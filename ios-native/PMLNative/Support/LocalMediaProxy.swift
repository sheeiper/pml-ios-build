import Foundation
import Network

/// 高性能 App 内直连媒体代理：
/// 1. 共享单 URLSession 连接池，复用 TCP/TLS 连接，告别每次拖动进度条重新握手；
/// 2. 自动缓存 AList `/d` 302 重定向后的真实 CDN 直链（过期/403 自动重取），省去每次 Range 探测的 302 往返；
/// 3. 监听 AVPlayer 断开事件（FIN/RST 与发送失败），毫秒级取消上游下载任务，杜绝后台僵尸流占满连接数；
/// 4. 纯异步背压控制（移除 15s DispatchSemaphore 阻塞），彻底解决连续拖动进度条或看几个视频后卡死的问题。
final class LocalMediaProxy: NSObject {
    static let shared = LocalMediaProxy()

    fileprivate final class StreamEntry {
        let token: String
        let direct: URL
        var resolvedURL: URL?
        var resolvedAt: Date?
        var lastAccess: Date = Date()

        init(token: String, direct: URL) {
            self.token = token
            self.direct = direct
        }
    }

    fileprivate let queue = DispatchQueue(label: "pml.local-proxy", qos: .userInitiated)
    private let lock = NSLock()
    private var streamsByToken: [String: StreamEntry] = [:]
    private var tokenByDirectURL: [String: String] = [:]
    private var listener: NWListener?
    private var port: UInt16?
    private var starting = false
    private var waiters: [CheckedContinuation<UInt16?, Never>] = []

    fileprivate let demux = ProxySessionDemux()
    fileprivate lazy var upstreamSession: URLSession = {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 20
        config.timeoutIntervalForResource = 3600
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        config.httpMaximumConnectionsPerHost = 8
        config.waitsForConnectivity = false
        let delegateQueue = OperationQueue()
        delegateQueue.name = "pml.local-proxy.upstream"
        delegateQueue.maxConcurrentOperationCount = 8
        delegateQueue.qualityOfService = .userInitiated
        return URLSession(configuration: config, delegate: demux, delegateQueue: delegateQueue)
    }()

    private override init() { super.init() }

    func register(directURL: URL) async -> URL? {
        guard let port = await ensureStarted() else { return nil }
        let token = upsertToken(for: directURL)
        let ext = directURL.pathExtension.isEmpty ? "mp4" : directURL.pathExtension
        return URL(string: "http://127.0.0.1:\(port)/p/\(token).\(ext)")
    }

    private func upsertToken(for directURL: URL) -> String {
        let key = directURL.absoluteString
        lock.lock()
        defer { lock.unlock() }
        if let existing = tokenByDirectURL[key], let entry = streamsByToken[existing] {
            entry.lastAccess = Date()
            return existing
        }
        let newToken = UUID().uuidString.replacingOccurrences(of: "-", with: "")
        let entry = StreamEntry(token: newToken, direct: directURL)
        streamsByToken[newToken] = entry
        tokenByDirectURL[key] = newToken
        pruneStreamsLocked()
        return newToken
    }

    private func pruneStreamsLocked() {
        guard streamsByToken.count > 96 else { return }
        let sorted = streamsByToken.values.sorted { $0.lastAccess < $1.lastAccess }
        for victim in sorted.prefix(streamsByToken.count - 64) {
            streamsByToken.removeValue(forKey: victim.token)
            tokenByDirectURL.removeValue(forKey: victim.direct.absoluteString)
        }
    }

    fileprivate func cachedTargetURL(for entry: StreamEntry) -> (url: URL, isCachedRedirect: Bool) {
        lock.lock()
        defer { lock.unlock() }
        entry.lastAccess = Date()
        if let resolved = entry.resolvedURL,
           let at = entry.resolvedAt,
           Date().timeIntervalSince(at) < 480 {
            return (resolved, true)
        }
        return (entry.direct, false)
    }

    fileprivate func updateResolvedURL(_ url: URL?, for entry: StreamEntry) {
        lock.lock()
        defer { lock.unlock() }
        if let url, url != entry.direct {
            entry.resolvedURL = url
            entry.resolvedAt = Date()
        } else if url == nil {
            entry.resolvedURL = nil
            entry.resolvedAt = nil
        }
    }

    private func currentPort() -> UInt16? {
        lock.lock()
        defer { lock.unlock() }
        return port
    }

    private func ensureStarted() async -> UInt16? {
        if let existing = currentPort() {
            return existing
        }
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
        if let tcpOptions = parameters.defaultProtocolStack.transportProtocol as? NWProtocolTCP.Options {
            tcpOptions.noDelay = true
        }
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

    private func streamEntry(for path: String) -> StreamEntry? {
        guard let token = path.split(separator: "/").last.map(String.init) else { return nil }
        let stem = token.split(separator: ".").first.map(String.init) ?? token
        lock.lock()
        defer { lock.unlock() }
        let entry = streamsByToken[stem]
        entry?.lastAccess = Date()
        return entry
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
        guard let entry = streamEntry(for: path) else {
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
            proxy: self,
            connection: connection,
            entry: entry,
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

/// 共享 URLSession 的任务分发器，避免每次 Range 请求创建独立 URLSession。
fileprivate final class ProxySessionDemux: NSObject, URLSessionDataDelegate {
    private let lock = NSLock()
    private var transfers: [Int: ProxyTransfer] = [:]

    func register(_ transfer: ProxyTransfer, for taskIdentifier: Int) {
        lock.lock()
        transfers[taskIdentifier] = transfer
        lock.unlock()
    }

    func unregister(taskIdentifier: Int) {
        lock.lock()
        transfers.removeValue(forKey: taskIdentifier)
        lock.unlock()
    }

    private func transfer(for task: URLSessionTask) -> ProxyTransfer? {
        lock.lock()
        defer { lock.unlock() }
        return transfers[task.taskIdentifier]
    }

    func urlSession(
        _ session: URLSession,
        dataTask: URLSessionDataTask,
        didReceive response: URLResponse,
        completionHandler: @escaping (URLSession.ResponseDisposition) -> Void
    ) {
        guard let transfer = transfer(for: dataTask) else {
            completionHandler(.cancel)
            return
        }
        transfer.handleResponse(dataTask: dataTask, response: response, completionHandler: completionHandler)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        guard let transfer = transfer(for: dataTask) else {
            dataTask.cancel()
            return
        }
        transfer.handleData(dataTask: dataTask, data: data)
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        let current = transfer(for: task)
        unregister(taskIdentifier: task.taskIdentifier)
        current?.handleComplete(task: task, error: error)
    }
}

fileprivate final class ProxyTransfer: NSObject {
    private weak var proxy: LocalMediaProxy?
    private let connection: NWConnection
    private let entry: LocalMediaProxy.StreamEntry
    private let method: String
    private let range: String?
    private let ifRange: String?

    private let stateLock = NSLock()
    private var task: URLSessionDataTask?
    private var usedCachedRedirect = false
    private var retriedDirect = false
    private var sentHeaders = false
    private var completed = false
    private var suspended = false
    private var queuedBytes = 0

    private static let highWaterMark = 1_572_864 // 1.5 MB
    private static let lowWaterMark = 524_288    // 512 KB

    init(
        proxy: LocalMediaProxy,
        connection: NWConnection,
        entry: LocalMediaProxy.StreamEntry,
        method: String,
        range: String?,
        ifRange: String?
    ) {
        self.proxy = proxy
        self.connection = connection
        self.entry = entry
        self.method = method
        self.range = range
        self.ifRange = ifRange
        super.init()
    }

    func start() {
        connection.stateUpdateHandler = { [weak self] state in
            switch state {
            case .failed, .cancelled:
                self?.abortTransfer()
            default:
                break
            }
        }
        watchClientDisconnect()

        guard let proxy else {
            abortTransfer()
            return
        }
        let target = proxy.cachedTargetURL(for: entry)
        usedCachedRedirect = target.isCachedRedirect
        startUpstream(url: target.url)
    }

    /// 监听 AVPlayer 主动关闭连接（例如拖动进度条放弃旧 Range 时发送 FIN）。
    private func watchClientDisconnect() {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 1024) { [weak self] _, _, isComplete, error in
            guard let self else { return }
            if isComplete || error != nil {
                self.abortTransfer()
            }
        }
    }

    private func startUpstream(url: URL) {
        guard let proxy else {
            abortTransfer()
            return
        }
        stateLock.lock()
        guard !completed else {
            stateLock.unlock()
            return
        }
        stateLock.unlock()

        var request = URLRequest(url: url)
        request.httpMethod = method
        request.timeoutInterval = 20
        request.cachePolicy = .reloadIgnoringLocalCacheData
        request.setValue("identity", forHTTPHeaderField: "Accept-Encoding")
        if let range { request.setValue(range, forHTTPHeaderField: "Range") }
        if let ifRange { request.setValue(ifRange, forHTTPHeaderField: "If-Range") }

        let newTask = proxy.upstreamSession.dataTask(with: request)
        stateLock.lock()
        if let old = task {
            proxy.demux.unregister(taskIdentifier: old.taskIdentifier)
            old.cancel()
        }
        task = newTask
        suspended = false
        queuedBytes = 0
        proxy.demux.register(self, for: newTask.taskIdentifier)
        stateLock.unlock()

        newTask.resume()
    }

    func handleResponse(
        dataTask: URLSessionDataTask,
        response: URLResponse,
        completionHandler: @escaping (URLSession.ResponseDisposition) -> Void
    ) {
        stateLock.lock()
        if completed {
            stateLock.unlock()
            completionHandler(.cancel)
            return
        }
        stateLock.unlock()

        guard let http = response as? HTTPURLResponse else {
            completionHandler(.cancel)
            fail()
            return
        }

        // 如果缓存的 302 CDN 直链过期或返回非 200/206，自动清缓存并回退原始 AList `/d` 重取一次
        if !(http.statusCode == 200 || http.statusCode == 206) {
            if (usedCachedRedirect || !retriedDirect), let proxy {
                usedCachedRedirect = false
                retriedDirect = true
                proxy.updateResolvedURL(nil, for: entry)
                proxy.demux.unregister(taskIdentifier: dataTask.taskIdentifier)
                completionHandler(.cancel)
                startUpstream(url: entry.direct)
                return
            }
            completionHandler(.cancel)
            fail()
            return
        }

        // 缓存 302 重定向后的最终 CDN 地址，加速后续所有 Seek Range 请求
        if let finalURL = http.url {
            proxy?.updateResolvedURL(finalURL, for: entry)
        }

        let headers = http.allHeaderFields.reduce(into: [String: String]()) { result, item in
            if let key = item.key as? String, let value = item.value as? String {
                result[key.lowercased()] = value
            }
        }
        var head = "HTTP/1.1 \(http.statusCode) \(HTTPURLResponse.localizedString(forStatusCode: http.statusCode))\r\n"
        head += "Connection: close\r\n"
        head += "Accept-Ranges: bytes\r\n"
        let passthrough = [
            ("content-type", "Content-Type"),
            ("content-length", "Content-Length"),
            ("content-range", "Content-Range"),
            ("last-modified", "Last-Modified"),
            ("etag", "ETag"),
        ]
        for (lower, canonical) in passthrough {
            if let value = headers[lower] {
                head += "\(canonical): \(value)\r\n"
            }
        }
        head += "\r\n"
        let payload = Data(head.utf8)

        stateLock.lock()
        sentHeaders = true
        stateLock.unlock()

        connection.send(content: payload, completion: .contentProcessed { [weak self] error in
            if error != nil {
                self?.abortTransfer()
            }
        })
        completionHandler(.allow)
        if method == "HEAD" {
            finish()
        }
    }

    func handleData(dataTask: URLSessionDataTask, data: Data) {
        let byteCount = data.count
        guard byteCount > 0 else { return }

        stateLock.lock()
        if completed {
            stateLock.unlock()
            dataTask.cancel()
            return
        }
        queuedBytes += byteCount
        let shouldSuspend = !suspended && queuedBytes > Self.highWaterMark
        if shouldSuspend {
            suspended = true
            dataTask.suspend()
        }
        stateLock.unlock()

        connection.send(content: data, completion: .contentProcessed { [weak self] error in
            guard let self else { return }
            if error != nil {
                self.abortTransfer()
                return
            }
            self.stateLock.lock()
            self.queuedBytes = max(0, self.queuedBytes - byteCount)
            let shouldResume = !self.completed && self.suspended && self.queuedBytes <= Self.lowWaterMark
            if shouldResume {
                self.suspended = false
                self.task?.resume()
            }
            self.stateLock.unlock()
        })
    }

    func handleComplete(task completedTask: URLSessionTask, error: Error?) {
        stateLock.lock()
        let isCurrent = (task?.taskIdentifier == completedTask.taskIdentifier)
        let alreadyCompleted = completed
        let hadSentHeaders = sentHeaders
        let canRetryDirect = !alreadyCompleted && !hadSentHeaders && (usedCachedRedirect || !retriedDirect)
        stateLock.unlock()

        guard isCurrent, !alreadyCompleted else { return }

        if error != nil {
            if canRetryDirect, let proxy {
                usedCachedRedirect = false
                retriedDirect = true
                proxy.updateResolvedURL(nil, for: entry)
                startUpstream(url: entry.direct)
                return
            }
            if !hadSentHeaders {
                fail()
                return
            }
        }
        finish()
    }

    fileprivate func abortTransfer() {
        stateLock.lock()
        guard !completed else {
            stateLock.unlock()
            return
        }
        completed = true
        let currentTask = task
        let wasSuspended = suspended
        suspended = false
        task = nil
        stateLock.unlock()

        if let currentTask {
            proxy?.demux.unregister(taskIdentifier: currentTask.taskIdentifier)
            if wasSuspended { currentTask.resume() }
            currentTask.cancel()
        }
        connection.stateUpdateHandler = nil
        connection.cancel()
    }

    private func finish() {
        stateLock.lock()
        guard !completed else {
            stateLock.unlock()
            return
        }
        completed = true
        let currentTask = task
        let wasSuspended = suspended
        suspended = false
        task = nil
        stateLock.unlock()

        if let currentTask {
            proxy?.demux.unregister(taskIdentifier: currentTask.taskIdentifier)
            if wasSuspended { currentTask.resume() }
        }
        connection.send(content: nil, contentContext: .finalMessage, isComplete: true, completion: .contentProcessed { [weak self] _ in
            self?.connection.stateUpdateHandler = nil
            self?.connection.cancel()
        })
    }

    private func fail() {
        stateLock.lock()
        guard !completed else {
            stateLock.unlock()
            return
        }
        completed = true
        let currentTask = task
        let wasSuspended = suspended
        suspended = false
        task = nil
        stateLock.unlock()

        if let currentTask {
            proxy?.demux.unregister(taskIdentifier: currentTask.taskIdentifier)
            if wasSuspended { currentTask.resume() }
            currentTask.cancel()
        }
        let body = Data("upstream failed".utf8)
        let head = "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\nContent-Length: \(body.count)\r\nConnection: close\r\n\r\n"
        var payload = Data(head.utf8)
        payload.append(body)
        connection.send(content: payload, completion: .contentProcessed { [weak self] _ in
            self?.connection.stateUpdateHandler = nil
            self?.connection.cancel()
        })
    }
}
