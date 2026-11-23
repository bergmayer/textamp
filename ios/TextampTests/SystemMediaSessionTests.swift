import AVFoundation
import MediaPlayer
import XCTest

@testable import Textamp

@MainActor
final class SystemMediaSessionTests: XCTestCase {
  private func player(
    _ key: String?, artwork: String = "cover", position: Double = 1500,
    playing: Bool = true
  ) throws -> Player {
    try JSONDecoder().decode(
      Player.self,
      from: JSONSerialization.data(withJSONObject: [
        "key": key as Any? ?? NSNull(), "title": key ?? "Nothing playing", "artist": "Artist",
        "album": "Album", "playing": playing, "position": position, "duration": 60000,
        "volume": 0, "artwork": artwork, "rows": [],
      ]))
  }

  private func waitFor(_ condition: @escaping () -> Bool) async throws {
    for _ in 0..<200 {
      if condition() { return }
      try await Task.sleep(for: .milliseconds(10))
    }
    XCTFail("Media session did not reach its expected state")
  }

  func testArtworkPublicationKeepsLatestClockAndRejectsStaleTrack() async throws {
    var requests: [String: CheckedContinuation<UIImage?, Never>] = [:]
    let center = MPNowPlayingInfoCenter.default()
    let session = SystemMediaSession(
      send: { _ in }, seek: { _ in },
      loadArtwork: { id in
        await withCheckedContinuation { requests[id] = $0 }
      })
    session.update(try player("old", artwork: "old-cover"))
    try await waitFor { requests["old-cover"] != nil }
    session.update(try player("new", artwork: "new-cover"))
    try await waitFor { requests["new-cover"] != nil }
    session.update(try player("new", artwork: "new-cover", position: 4250, playing: false))
    let image = UIGraphicsImageRenderer(size: CGSize(width: 600, height: 600)).image { context in
      UIColor.red.setFill()
      context.fill(CGRect(x: 0, y: 0, width: 600, height: 600))
    }
    requests.removeValue(forKey: "new-cover")?.resume(returning: image)
    try await waitFor { center.nowPlayingInfo?[MPMediaItemPropertyArtwork] != nil }
    requests.removeValue(forKey: "old-cover")?.resume(returning: UIImage())
    await Task.yield()
    XCTAssertEqual(center.nowPlayingInfo?[MPMediaItemPropertyTitle] as? String, "new")
    XCTAssertEqual(
      center.nowPlayingInfo?[MPNowPlayingInfoPropertyElapsedPlaybackTime] as? Double, 4.25)
    XCTAssertEqual(center.nowPlayingInfo?[MPNowPlayingInfoPropertyPlaybackRate] as? Double, 0)
    let art = try XCTUnwrap(
      center.nowPlayingInfo?[MPMediaItemPropertyArtwork] as? MPMediaItemArtwork)
    XCTAssertEqual(art.bounds.size, image.size)
    XCTAssertNotNil(art.image(at: CGSize(width: 300, height: 300)))
    session.update(try player(nil))
    XCTAssertNil(center.nowPlayingInfo)
    XCTAssertFalse(MPRemoteCommandCenter.shared().playCommand.isEnabled)
  }

  func testRemoteCommandsAndHeadphoneRemovalUseExplicitPause() async throws {
    var operations: [String] = []
    let session = SystemMediaSession(
      send: { operations.append($0) }, seek: { _ in },
      loadArtwork: { _ in nil })
    session.update(try player("track"))
    let commands = MPRemoteCommandCenter.shared()
    XCTAssertTrue(commands.playCommand.isEnabled)
    XCTAssertTrue(commands.pauseCommand.isEnabled)
    XCTAssertTrue(commands.togglePlayPauseCommand.isEnabled)
    XCTAssertTrue(commands.changePlaybackPositionCommand.isEnabled)
    session.perform("pause")
    XCTAssertEqual(operations, ["pause"])
    NotificationCenter.default.post(
      name: AVAudioSession.routeChangeNotification, object: nil,
      userInfo: [
        AVAudioSessionRouteChangeReasonKey:
          AVAudioSession.RouteChangeReason.oldDeviceUnavailable.rawValue
      ])
    try await waitFor { operations.count == 2 }
    XCTAssertEqual(operations, ["pause", "output_interrupted"])
    session.update(try player(nil))
  }

  func testInterruptionResumesOnlyWithPermissionAndMediaResetWaitsForPlay() async throws {
    var operations: [String] = []
    let session = SystemMediaSession(
      send: { operations.append($0) }, seek: { _ in }, loadArtwork: { _ in nil })
    session.update(try player("track"))
    let center = NotificationCenter.default
    center.post(name: AVAudioSession.interruptionNotification, object: nil, userInfo: [
      AVAudioSessionInterruptionTypeKey: AVAudioSession.InterruptionType.began.rawValue
    ])
    try await waitFor { operations.count == 1 }
    center.post(name: AVAudioSession.interruptionNotification, object: nil, userInfo: [
      AVAudioSessionInterruptionTypeKey: AVAudioSession.InterruptionType.ended.rawValue,
      AVAudioSessionInterruptionOptionKey: AVAudioSession.InterruptionOptions.shouldResume.rawValue
    ])
    try await waitFor { operations.count == 2 }
    center.post(name: AVAudioSession.mediaServicesWereResetNotification, object: nil)
    try await waitFor { operations.count == 3 }
    XCTAssertEqual(operations, ["output_interrupted", "play", "output_interrupted"])
    session.update(try player(nil))
  }

  func testManualPauseDuringInterruptionCancelsAutomaticResume() async throws {
    var operations: [String] = []
    let session = SystemMediaSession(
      send: { operations.append($0) }, seek: { _ in },
      loadArtwork: { _ in nil })
    session.update(try player("track"))
    NotificationCenter.default.post(
      name: AVAudioSession.interruptionNotification, object: nil,
      userInfo: [AVAudioSessionInterruptionTypeKey: AVAudioSession.InterruptionType.began.rawValue])
    try await waitFor { operations.count == 1 }
    session.perform("pause")
    NotificationCenter.default.post(
      name: AVAudioSession.interruptionNotification, object: nil,
      userInfo: [
        AVAudioSessionInterruptionTypeKey: AVAudioSession.InterruptionType.ended.rawValue,
        AVAudioSessionInterruptionOptionKey: AVAudioSession.InterruptionOptions.shouldResume
          .rawValue,
      ])
    await Task.yield()
    XCTAssertEqual(operations, ["output_interrupted", "pause"])
    session.update(try player(nil))
  }

  func testInterruptionWithoutResumePermissionDoesNotResumeLater() async throws {
    var operations: [String] = []
    let session = SystemMediaSession(
      send: { operations.append($0) }, seek: { _ in },
      loadArtwork: { _ in nil })
    session.update(try player("track"))
    NotificationCenter.default.post(
      name: AVAudioSession.interruptionNotification, object: nil,
      userInfo: [AVAudioSessionInterruptionTypeKey: AVAudioSession.InterruptionType.began.rawValue])
    try await waitFor { operations.count == 1 }
    NotificationCenter.default.post(
      name: AVAudioSession.interruptionNotification, object: nil,
      userInfo: [AVAudioSessionInterruptionTypeKey: AVAudioSession.InterruptionType.ended.rawValue])
    await Task.yield()
    NotificationCenter.default.post(
      name: AVAudioSession.interruptionNotification, object: nil,
      userInfo: [
        AVAudioSessionInterruptionTypeKey: AVAudioSession.InterruptionType.ended.rawValue,
        AVAudioSessionInterruptionOptionKey: AVAudioSession.InterruptionOptions.shouldResume
          .rawValue,
      ])
    await Task.yield()
    XCTAssertEqual(operations, ["output_interrupted"])
    session.update(try player(nil))
  }
}
