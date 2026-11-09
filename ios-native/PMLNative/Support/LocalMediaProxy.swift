import Foundation
import Network

/// 最小 App 内视频代理：AVPlayer 只访问 127.0.0.1，
/// 代理负责跟随 AList /d 的 302、转发 Range，并在失败时回退服务器中转。
final class LocalMediaProxy: NSObject {
    static let shared = LocalMediaProxy()

    private struct Stream {
        let direct: URL
        let fallback: URL?
        let fallbackHeaders: [String: String]
    }

    private let queue = DispatchQueue(label: "pml.local-proxy")
    private let lock = NSLock()
    private var streams: [String: Stream] = [:]
    private var listener: NWListener?
    private var port: UInt16?
    private var starting = false
    private var waiters: [CheckedContinuation<UInt16?, Never>] = []

    private override init() { super.init() }

    func register(directURL: URL, fallbackURL: URL?, fallbackHeaders: [String: String]) async -> URL? {
        guard let port = await ensureStarted() else { return nil }
        let token = UUID().uuidString.replacingOccurrences(of: "-", with: "")
        lock.lock()
        streams[token] = Stream(direct: directURL, fallback: fallbackURL, fallbackHeaders: fallbackHeaders)
        lock.unlock()
        let ext = directURL.pathExtension.isEmpty ? "mp4" : directURL.pathExtension
        return URL(string: "http://127.0.0.1:\(port)/p/\(token).\(ext)")
    }

    private func ensureStarted() async -> UInt16? {
        if let port { return port }
        return await withCheckedContinuation { continuation in
            lock.lock()
            if let port {
                lock.unlock()
                continuation.resume(returning: port)
                return
            }
            waiters.append(continuation)
            let shouldStart = !starting
            starting = true
            lock.unlock()
            if shouldStart { startListener() }
        }
    }

    private func startListener() {
        let parameters = NWParameters.tcp
        parameters.allowLocalEndpointReuse = true
        parameters.requiredLocalEndpoint = NWEndpoint.hostPort(host: .ipv4(.loopback), port: .any)
        do {
            let listener = try NWListener(using: parameters, on: .any)
            self.listener = listener
            listener.stateUpdateHandler = { [weak self] state in
                guard let self else { return }
                switch state {
                case .ready:
                    self.finishStart(port: listener.port?.rawValue)
                case .failed, .cancelled:
                    self.finishStart(port: nil)
                default:
                    break
                }
            }
            listener.newConnectionHandler = { [weak self] connection in
                self?.accept(connection)
            }
            listener.start(queue: queue)
        } catch {
            finishStart(port: nil)
        }
    }

    private func finishStart(port: UInt16?) {
        lock.lock()
        if self.port == nil { self.port = port }
        let currentPort = self.port
        let currentWaiters = waiters
        waiters.removeAll()
        starting = false
        lock.unlock()
        for waiter in currentWaiters { waiter.resume(returning: currentPort) }
    }

    private func stream(for path: String) -> Stream? {
        guard let token = path.split(separator: "/").last.map(String.init) else { return nil }
        let stem = token.split(separator: ".").first.map(String.init) ?? token
        lock.lock()
        defer { lock.unlock() }
        return streams[stem]
    }

    private func accept(_ connection: NWConnection) {
        connection.start(queue: queue)
        receive(connection, buffer: Data())
    }

    private func receive(_ connection: NWConnection, buffer: Data) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 32 * 1024) { [weak self] data, _, isComplete, error in
            guard let self else { connection.cancel(); return }
            var accumulated = buffer
            if let data { accumulated.append(data) }
            if let headerEnd = accumulated.range(of: Data("\r\n\r\n".utf8)) {
                let headerData = accumulated.subdata(in: 0..<headerEnd.lowerBound)
                self.handle(connection, header: headerData)
                return
            }
            if isComplete || error != nil || accumulated.count > 64 * 1024 {
                connection.cancel()
            } else {
                self.receive(connection, buffer: accumulated)
            }
        }
    }

    private func handle(_ connection: NWConnection, header: Data) {
        guard let text = String(data: header, encoding: .utf8) else { connection.cancel(); return }
        let lines = text.components(separatedBy: "\r\n")
        guard let requestLine = lines.first else { connection.cancel(); return }
        let parts = requestLine.split(separator: " ")
        guard parts.count >= 2 else { connection.cancel(); return }
        let method = String(parts[0])
        let path = String(parts[1]).split(separator: "?").first.map(String.init) ?? String(parts[1])
        guard method == "GET" || method == "HEAD" else {
            respond(connection, status: "405 Method Not Allowed", headers: [:], body: Data())
            return
        }
        guard let stream = stream(for: path) else {
            respond(connection, status: "404 Not Found", headers: ["Content-Type": "text/plain"], body: Data("not found".utf8))
            return
        }
        var range: String?
        var ifRange: String?
        for line in lines.dropFirst() {
            guard let colon = line.firstIndex(of: ":") else { continue }
            let name = line[..<colon].trimmingCharacters(in: .whitespaces).lowercased()
            let value = line[line.index(after: colon)...].trimmingCharacters(in: .whitespaces)
            if name == "range" { range = value }
            if name == "if-range" { ifRange = value }
        }
        let transfer = ProxyTransfer(
            connection: connection,
            stream: stream,
            method: method,
            range: range,
            ifRange: ifRange
        )
        transfer.start()
    }

    private func respond(_ connection: NWConnection, status: String, headers: [String: String], body: Data) {
        var head = "HTTP/1.1 \(status)\r\n"
        for (key, value) in headers { head += "\(key): \(value)\r\n" }
        head += "Content-Length: \(body.count)\r\nConnection: close\r\n\r\n"
        var payload = Data(head.utf8)
        payload.append(body)
        connection.send(content: payload, completion: .contentProcessed { _ in connection.cancel() })
    }
}

private final class ProxyTransfer: NSObject, URLSessionDataDelegate {
    private let connection: NWConnection
    private let stream: LocalMediaProxyStreamBox
    private let method: String
    private let range: String?
    private let ifRange: String?
    private var session: URLSession?
    private var task: URLSessionDataTask?
    private var usedFallback = false
    private var sentHeaders = false
    private var completed = false

    init(connection: NWConnection, stream: LocalMediaProxyStreamBox, method: String, range: String?, ifRange: String?) {
        self.connection = connection
        self.stream = stream
        self.method = method
        self.range = range
        self.ifRange = ifRange
        super.init()
    }

    func start() {
        request(url: stream.direct, headers: [:])
    }

    private func request(url: URL, headers: [String: String]) {
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.timeoutInterval = 30
        request.cachePolicy = .reloadIgnoringLocalCacheData
        request.setValue("identity", forHTTPHeaderField: "Accept-Encoding")
        if let range { request.setValue(range, forHTTPHeaderField: "Range") }
        if let ifRange { request.setValue(ifRange, forHTTPHeaderField: "If-Range") }
        for (key, value) in headers { request.setValue(value, forHTTPHeaderField: key) }

        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 30
        configuration.timeoutIntervalForResource = 3600
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        let session = URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
        self.session = session
        let task = session.dataTask(with: request)
        self.task = task
        task.resume()
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse, completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        guard let http = response as? HTTPURLResponse else {
            completionHandler(.cancel)
            fail()
            return
        }
        if !usedFallback, [401, 403, 404, 500, 502, 503, 504].contains(http.statusCode), let fallback = stream.fallback {
            usedFallback = true
            completionHandler(.cancel)
            session.invalidateAndCancel()
            request(url: fallback, headers: stream.fallbackHeaders)
            return
        }
        guard http.statusCode == 200 || http.statusCode == 206 else {
            completionHandler(.cancel)
            fail()
            return
        }
        let headers = http.allHeaderFields.reduce(into: [String: String]()) { result, item in
            if let key = item.key as? String, let value = item.value as? String {
                result[key] = value
            }
        }
        var head = "HTTP/1.1 \(http.statusCode) \(HTTPURLResponse.localizedString(forStatusCode: http.statusCode))\r\n"
        head += "Connection: close\r\n"
        head += "Accept-Ranges: bytes\r\n"
        for name in ["Content-Type", "Content-Length", "Content-Range", "Last-Modified", "ETag"] {
            if let value = headers[name] { head += "\(name): \(value)\r\n" }
        }
        head += "\r\n"
        let payload = Data(head.utf8)
        connection.send(content: payload, completion: .contentProcessed { _ in })
        sentHeaders = true
        completionHandler(.allow)
        if method == "HEAD" { finish() }
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        guard !completed else { return }
        let semaphore = DispatchSemaphore(value: 0)
        connection.send(content: data, completion: .contentProcessed { _ in semaphore.signal() })
        _ = semaphore.wait(timeout: .now() + 15)
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        if let error, !sentHeaders {
            _ = error
            fail()
            return
        }
        finish()
    }

    private func finish() {
        guard !completed else { return }
        completed = true
        session?.finishTasksAndInvalidate()
        session = nil
        connection.send(content: nil, contentContext: .finalMessage, isComplete: true, completion: .contentProcessed { _ in
            self.connection.cancel()
        })
    }

    private func fail() {
        guard !completed else { return }
        completed = true
        session?.invalidateAndCancel()
        session = nil
        let body = Data("upstream failed".utf8)
        var head = "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\nContent-Length: \(body.count)\r\nConnection: close\r\n\r\n"
        var payload = Data(head.utf8)
        head = ""
        payload.append(body)
        connection.send(content: payload, completion: .contentProcessed { _ in self.connection.cancel() })
    }
}

/// 暴露给 ProxyTransfer 的流配置（避免嵌套类型可见性问题）。
struct LocalMediaProxyStreamBox {
    let direct: URL
    let fallback: URL?
    let fallbackHeaders: [String: String]
}

private extension LocalMediaProxy {
    /// Bridge private Stream to the transfer box.
    private func box(for stream: Stream) -> LocalMediaProxyStreamBox {
        LocalMediaProxyStreamBox(direct: stream.direct, fallback: stream.fallback, fallbackHeaders: stream.fallbackHeaders)
    }
}
