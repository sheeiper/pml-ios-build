import Foundation

/// Result of one real provider check for a domain root.
struct DomainRefreshOutcome: Identifiable, Sendable {
    let domain: String
    let scanned: Int64
    let added: Int64
    let updated: Int64
    let removed: Int64

    var id: String { domain }
    var changed: Int64 { added + updated + removed }
    var summaryText: String {
        if changed == 0 { return "\(domain) 无变化" }
        return "\(domain)：新增 \(added)、更新 \(updated)、移除 \(removed)"
    }
}

extension PMLAPIClient {
    /// Shared by pull-to-refresh and the toolbar refresh button so both paths
    /// really contact the provider (CD2/AList) before any cached data is shown.
    func checkDomainRootsForUpdates(
        domains: [String] = ["recent", "pending", "vault", "comics"]
    ) async throws -> [DomainRefreshOutcome] {
        let summaries = try await domainSummaries()
        var outcomes: [DomainRefreshOutcome] = []
        var firstError: Error?
        for domain in domains {
            guard let summary = summaries.first(where: { $0.domain == domain }),
                  let rootPath = summary.rootPath?.trimmingCharacters(in: .whitespaces),
                  !rootPath.isEmpty else { continue }
            do {
                let scan = try await refreshDomainPath(domain: domain, path: rootPath)
                outcomes.append(
                    DomainRefreshOutcome(
                        domain: domain,
                        scanned: scan.scanned,
                        added: scan.added,
                        updated: scan.updated,
                        removed: scan.deleted
                    )
                )
            } catch {
                if firstError == nil { firstError = error }
            }
        }
        // A partial failure still returns the successful checks, but never hides
        // a provider error behind a silent success.
        if let firstError, outcomes.isEmpty { throw firstError }
        return outcomes
    }
}

extension Array where Element == DomainRefreshOutcome {
    var refreshNotice: String {
        guard !isEmpty else { return "没有可检查的根目录" }
        let changed = reduce(Int64(0)) { $0 + $1.changed }
        if changed == 0 { return "已检查 CD2：暂无更新" }
        return "已检查 CD2：" + map(\.summaryText).joined(separator: "，")
    }
}
