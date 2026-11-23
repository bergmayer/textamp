import AVFoundation
import MediaPlayer
import UIKit

/// Connects the shared player's state and commands to iOS media surfaces and accessories.
@MainActor
final class SystemMediaSession {
  private let send: (String) -> Void
  private let seek: (UInt64) -> Void
  private let loadArtwork: (String) async -> UIImage?
  private let center: MPNowPlayingInfoCenter
  private let commands: MPRemoteCommandCenter
  private var targets: [(MPRemoteCommand, Any)] = []
  private var observers: [NSObjectProtocol] = []
  private var player: Player?
  private var publishedPosition: (milliseconds: Double, uptime: TimeInterval)?
  private var artworkID: String?
  private var artwork: MPMediaItemArtwork?
  private var artworkTask: Task<Void, Never>?
  private var resumeAfterInterruption = false

  init(
    send: @escaping (String) -> Void,
    seek: @escaping (UInt64) -> Void,
    loadArtwork: @escaping (String) async -> UIImage?,
    center: MPNowPlayingInfoCenter = .default(),
    commands: MPRemoteCommandCenter = .shared()
  ) {
    self.send = send
    self.seek = seek
    self.loadArtwork = loadArtwork
    self.center = center
    self.commands = commands
    for (command, operation) in [
      (commands.playCommand, "play"), (commands.pauseCommand, "pause"),
      (commands.togglePlayPauseCommand, "play_pause"), (commands.nextTrackCommand, "next"),
      (commands.previousTrackCommand, "previous"), (commands.stopCommand, "stop"),
    ] {
      let token = command.addTarget { [weak self] _ in
        Task { @MainActor in self?.perform(operation) }
        return .success
      }
      targets.append((command, token))
    }
    let token = commands.changePlaybackPositionCommand.addTarget { [weak self] event in
      guard let event = event as? MPChangePlaybackPositionCommandEvent,
        event.positionTime.isFinite, event.positionTime >= 0,
        event.positionTime < Double(UInt64.max) / 1000
      else { return .commandFailed }
      Task { @MainActor in self?.seek(UInt64(event.positionTime * 1000)) }
      return .success
    }
    targets.append((commands.changePlaybackPositionCommand, token))
    observe(AVAudioSession.interruptionNotification) { session, notification in
      guard let raw = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
        let type = AVAudioSession.InterruptionType(rawValue: raw)
      else { return }
      switch type {
      case .began:
        session.resumeAfterInterruption = session.player?.playing == true
        session.send("output_interrupted")
      case .ended:
        let options = notification.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt ?? 0
        let resume =
          session.resumeAfterInterruption
          && AVAudioSession.InterruptionOptions(rawValue: options).contains(.shouldResume)
        session.resumeAfterInterruption = false
        if resume { session.perform("play") }
      @unknown default: break
      }
    }
    observe(AVAudioSession.routeChangeNotification) { session, notification in
      guard let reason = notification.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt,
        reason == AVAudioSession.RouteChangeReason.oldDeviceUnavailable.rawValue
      else { return }
      session.resumeAfterInterruption = false
      session.send("output_interrupted")
    }
    observe(AVAudioSession.mediaServicesWereResetNotification) { session, _ in
      session.resumeAfterInterruption = false
      session.send("output_interrupted")
    }
    updateCommands()
  }

  func perform(_ operation: String) {
    resumeAfterInterruption = false
    send(operation)
  }

  func update(_ player: Player) {
    let previousKey = self.player?.key
    self.player = player
    if previousKey != player.key { resumeAfterInterruption = false }
    updateCommands()
    guard player.key != nil else {
      artworkTask?.cancel()
      artworkTask = nil
      artworkID = nil
      artwork = nil
      center.nowPlayingInfo = nil
      return
    }
    if artworkID != player.artwork || previousKey != player.key {
      artworkTask?.cancel()
      artworkID = player.artwork
      artwork = nil
      let id = player.artwork
      let key = player.key
      artworkTask = Task { [weak self, loadArtwork] in
        let image = await loadArtwork(id)
        guard !Task.isCancelled, let self, self.player?.key == key, self.artworkID == id,
          let image
        else { return }
        self.artwork = MPMediaItemArtwork(boundsSize: image.size) { _ in image }
        self.publish()
      }
    }
    publish()
  }

  func updatePosition(_ milliseconds: Double) {
    guard let current = player, current.key != nil else { return }
    player?.position = milliseconds
    let now = ProcessInfo.processInfo.systemUptime
    if let last = publishedPosition {
      let expected = last.milliseconds + (current.playing ? (now - last.uptime) * 1000 : 0)
      if abs(expected - milliseconds) < 500 { return }
    }
    publish()
  }

  private func publish() {
    guard let player, let key = player.key else { return }
    publishedPosition = (player.position, ProcessInfo.processInfo.systemUptime)
    var info: [String: Any] = [
      MPMediaItemPropertyTitle: player.title,
      MPMediaItemPropertyArtist: player.artist,
      MPMediaItemPropertyAlbumTitle: player.album,
      MPMediaItemPropertyMediaType: MPMediaType.music.rawValue,
      MPNowPlayingInfoPropertyMediaType: MPNowPlayingInfoMediaType.audio.rawValue,
      MPNowPlayingInfoPropertyExternalContentIdentifier: key,
      MPMediaItemPropertyPlaybackDuration: player.duration / 1000,
      MPNowPlayingInfoPropertyElapsedPlaybackTime: player.position / 1000,
      MPNowPlayingInfoPropertyDefaultPlaybackRate: 1.0,
      MPNowPlayingInfoPropertyPlaybackRate: player.playing ? 1.0 : 0.0,
      MPNowPlayingInfoPropertyIsLiveStream: false,
    ]
    if let artist = player.albumArtist { info[MPMediaItemPropertyAlbumArtist] = artist }
    if let artwork { info[MPMediaItemPropertyArtwork] = artwork }
    center.nowPlayingInfo = info
  }

  private func updateCommands() {
    let available = player?.key != nil
    for (command, _) in targets { command.isEnabled = available }
    commands.changePlaybackPositionCommand.isEnabled = available && (player?.duration ?? 0) > 0
  }

  private func observe(
    _ name: Notification.Name,
    handler: @escaping (SystemMediaSession, Notification) -> Void
  ) {
    observers.append(
      NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) {
        [weak self] notification in
        Task { @MainActor in
          guard let self else { return }
          handler(self, notification)
        }
      })
  }

  deinit {
    artworkTask?.cancel()
    for (command, token) in targets { command.removeTarget(token) }
    for observer in observers { NotificationCenter.default.removeObserver(observer) }
  }
}
