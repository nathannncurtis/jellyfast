// AVFoundation playability checker.
// Usage: avcheck [timeout_secs]   (URL is read from stdin so tokens never appear in argv)
// Output (one line): "READY played=<true|false> t=<secs>" or "FAILED <error>" or "TIMEOUT".
// Build: xcrun swiftc -O tools/avcheck.swift -o <out>
import AVFoundation
import Foundation

let timeout = CommandLine.arguments.count > 1 ? (Double(CommandLine.arguments[1]) ?? 30) : 30
guard let line = readLine(), let url = URL(string: line.trimmingCharacters(in: .whitespacesAndNewlines)) else {
    print("FAILED bad url on stdin"); exit(2)
}
let asset = AVURLAsset(url: url)
let item = AVPlayerItem(asset: asset)
let player = AVPlayer(playerItem: item)
player.isMuted = true
func spin(_ s: Double) { RunLoop.main.run(until: Date(timeIntervalSinceNow: s)) }

let start = Date()
while item.status == .unknown && Date().timeIntervalSince(start) < timeout { spin(0.1) }
switch item.status {
case .failed:
    let e = item.error as NSError?
    var msg = e?.localizedDescription ?? "unknown"
    if let e = e { msg += " [\(e.domain) \(e.code)]" }
    if let log = item.errorLog()?.events.last { msg += " http=\(log.errorStatusCode) \(log.errorComment ?? "")" }
    print("FAILED \(msg)"); exit(1)
case .unknown:
    print("TIMEOUT"); exit(1)
default: break
}
player.play()
let t0 = Date()
var t = 0.0
while Date().timeIntervalSince(t0) < min(timeout, 15) {
    spin(0.2)
    t = player.currentTime().seconds
    if t.isFinite && t > 0.5 { break }
    if item.status == .failed {
        print("FAILED during playback: \(item.error?.localizedDescription ?? "?")"); exit(1)
    }
}
let ok = t.isFinite && t > 0.5
print("READY played=\(ok) t=\(String(format: "%.1f", t.isFinite ? t : 0))")
exit(ok ? 0 : 1)
