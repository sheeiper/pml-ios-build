import Foundation

/// 与 Rust 端 `POST /api/{command}` 协议保持一致（Basic Auth + JSON）。
/// 网页版、桌面版与原生端共用同一套命令语义。
struct PMLAPIClient {
    struct Configuration {
        let baseURL: URL
        let username: String
        let password: String
    }

    enum APIError: LocalizedError {
        case badURL
        case unauthorized
        case malformedResponse
        case server(String)

        var errorDescription: String? {
            switch self {
            case .badURL:
                return "服务地址无效，请在设置里填写完整的 http(s) 地址"
            case .unauthorized:
                return "登录失败：账号或密码错误"
            case .malformedResponse:
                return "服务器返回了无法解析的响应"
            case .server(let message):
                return message
            }
        }
    }

    let configuration: Configuration



    /// App 前后台切换后可能残留失效连接，对可重试错误做退避重试。
    private static func send(_ request: URLRequest, isThumbnail: Bool = false) async throws -> (Data, URLResponse) {
        var lastError: Error?
        let maxAttempts = isThumbnail ? 2 : 3
        for attempt in 0..<maxAttempts {
            do {
                let targetSession = isThumbnail ? APISession.shared.thumbnailSession : APISession.shared.session
                return try await targetSession.data(for: request)
            } catch {
                let ns = error as NSError
                let retriable = ns.domain == NSURLErrorDomain && [
                    NSURLErrorCannotConnectToHost,
                    NSURLErrorNetworkConnectionLost,
                    NSURLErrorTimedOut,
                    NSURLErrorCannotFindHost,
                ].contains(ns.code)
                if !retriable || attempt == maxAttempts - 1 { throw error }
                lastError = error
                if !isThumbnail {
                    APISession.shared.reset()
                }
                try? await Task.sleep(nanoseconds: UInt64(250_000_000 * (attempt + 1)))
            }
        }
        throw lastError ?? APIError.server("请求失败")
    }

    func command<T: Decodable>(
        _ name: String,
        arguments: [String: Any] = [:],
        as type: T.Type = T.self
    ) async throws -> T {
        let data = try await rawCommand(name, arguments: arguments)
        do {
            return try JSONDecoder().decode(T.self, from: data)
        } catch {
            throw APIError.server("解析 \(name) 响应失败：\(error.localizedDescription)")
        }
    }

    func command(_ name: String, arguments: [String: Any] = [:]) async throws {
        _ = try await rawCommand(name, arguments: arguments)
    }

    /// 调用无返回值的命令（结果体被忽略）。
    func perform(_ name: String, arguments: [String: Any] = [:]) async throws {
        _ = try await rawCommand(name, arguments: arguments)
    }

    /// GET /api/health，用于确认内嵌内核是否真的在运行。
    func health() async throws -> String {
        guard var components = URLComponents(url: configuration.baseURL, resolvingAgainstBaseURL: false) else {
            throw APIError.badURL
        }
        components.path = "/api/health"
        components.query = nil
        guard let url = components.url else { throw APIError.badURL }

        var request = URLRequest(url: url, timeoutInterval: 10)
        request.httpMethod = "GET"
        let credential = Data("\(configuration.username):\(configuration.password)".utf8).base64EncodedString()
        request.setValue("Basic \(credential)", forHTTPHeaderField: "Authorization")

        let (data, response) = try await Self.send(request, isThumbnail: false)
        guard let http = response as? HTTPURLResponse else { throw APIError.malformedResponse }
        guard (200..<300).contains(http.statusCode) else {
            throw APIError.server(String(data: data, encoding: .utf8) ?? "HTTP \(http.statusCode)")
        }
        struct HealthResponse: Decodable { let status: String }
        return (try? JSONDecoder().decode(HealthResponse.self, from: data))?.status ?? "ok"
    }

    private func rawCommand(_ name: String, arguments: [String: Any]) async throws -> Data {
        guard var components = URLComponents(url: configuration.baseURL, resolvingAgainstBaseURL: false) else {
            throw APIError.badURL
        }
        components.path = "/api/" + name
        components.query = nil
        guard let url = components.url else { throw APIError.badURL }

        let timeout: TimeInterval = {
            switch name {
            case "scan_domain": return 7200
            case "create_pending_project": return 600
            default: return 30
            }
        }()
        var request = URLRequest(url: url, timeoutInterval: timeout)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")

        let credential = Data("\(configuration.username):\(configuration.password)".utf8).base64EncodedString()
        request.setValue("Basic \(credential)", forHTTPHeaderField: "Authorization")
        request.httpBody = try JSONSerialization.data(withJSONObject: arguments)

        let isThumbnail = (name == "get_thumbnail_input" || name == "get_asset_thumbnail" || name == "save_asset_thumbnail")
        let (data, response) = try await Self.send(request, isThumbnail: isThumbnail)
        guard let http = response as? HTTPURLResponse else { throw APIError.malformedResponse }
        if http.statusCode == 401 { throw APIError.unauthorized }
        guard (200..<300).contains(http.statusCode) else {
            // Rust 端错误体是 {"error": "..."}，直接展示可读文案而不是原始 JSON。
            if let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let message = object["error"] as? String,
               !message.isEmpty {
                throw APIError.server(message)
            }
            throw APIError.server(String(data: data, encoding: .utf8) ?? "HTTP \(http.statusCode)")
        }
        return data
    }
}
