import Foundation

struct SubtitleCue: Identifiable {
    let id = UUID()
    let start: TimeInterval
    let end: TimeInterval
    let text: String
}

enum SubtitleParser {
    static func parse(_ raw: String) -> [SubtitleCue] {
        var text = raw
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
        if text.hasPrefix("\u{FEFF}") { text.removeFirst() }

        var cues: [SubtitleCue] = []
        for block in text.components(separatedBy: "\n\n") {
            let lines = block
                .split(separator: "\n", omittingEmptySubsequences: false)
                .map(String.init)
            guard let timeIndex = lines.firstIndex(where: { $0.contains("-->") }) else { continue }
            let timeLine = lines[timeIndex]
                .replacingOccurrences(of: ",", with: ".")
                .replacingOccurrences(of: "，", with: ".")
            let parts = timeLine.components(separatedBy: "-->")
            guard parts.count >= 2,
                  let start = parseTimestamp(parts[0]),
                  let end = parseTimestamp(parts[1]) else { continue }
            let content = lines
                .dropFirst(timeIndex + 1)
                .joined(separator: "\n")
                .replacingOccurrences(of: "<[^>]+>", with: "", options: .regularExpression)
                .replacingOccurrences(of: "&nbsp;", with: " ")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if !content.isEmpty, end > start {
                cues.append(SubtitleCue(start: start, end: end, text: content))
            }
        }
        return cues.sorted { $0.start < $1.start }
    }

    static func cue(at time: TimeInterval, in cues: [SubtitleCue]) -> SubtitleCue? {
        guard !cues.isEmpty else { return nil }
        var low = 0
        var high = cues.count - 1
        var answer: SubtitleCue?
        while low <= high {
            let mid = (low + high) / 2
            let cue = cues[mid]
            if cue.start <= time {
                answer = cue
                low = mid + 1
            } else {
                high = mid - 1
            }
        }
        guard let answer, time <= answer.end + 0.3 else { return nil }
        return answer
    }

    private static func parseTimestamp(_ raw: String) -> TimeInterval? {
        let token = raw
            .trimmingCharacters(in: .whitespaces)
            .split(separator: " ")
            .first
            .map(String.init) ?? raw
        let pieces = token.split(separator: ":")
        guard pieces.count >= 2,
              let seconds = Double(pieces[pieces.count - 1].replacingOccurrences(of: ",", with: ".")) else {
            return nil
        }
        let minutes = Double(pieces[pieces.count - 2]) ?? 0
        let hours = pieces.count >= 3 ? (Double(pieces[pieces.count - 3]) ?? 0) : 0
        return hours * 3600 + minutes * 60 + seconds
    }
}