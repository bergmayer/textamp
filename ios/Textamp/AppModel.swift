import Network
import OSLog
import Observation
import SwiftUI

@MainActor
@Observable
final class AppModel {
  var snapshot: Snapshot?
  var player: Player?
  var error: String? {
    didSet {
      if let error { bridge.recordError(error) }
    }
  }
  var tab = "library"
  var libraryTab = "library"
  var sheet: String?
  var settings = false
  var path: [String] = []
  let bridge = CoreBridge()
  private let network = NWPathMonitor()
  private var mediaSession: SystemMediaSession?
  private var iconTask: Task<Void, Never>?

  init() {
    bridge.onUpdate = { [weak self] update in DispatchQueue.main.async { self?.apply(update) } }
    bridge.onError = { [weak self] error in DispatchQueue.main.async { self?.error = error } }
    mediaSession = SystemMediaSession(
      send: { [weak self] operation in self?.send("transport", ["operation": operation]) },
      seek: { [weak self] milliseconds in self?.send("seek", ["milliseconds": milliseconds]) },
      loadArtwork: { [bridge] id in
        await ArtworkCache.shared.image(id: id, mode: "auto", bridge: bridge)
      })
    bridge.start()
    network.pathUpdateHandler = { [weak self] path in
      Task { @MainActor in self?.send("connectivity", ["available": path.status == .satisfied]) }
    }
    network.start(queue: DispatchQueue(label: "textamp.connectivity"))
  }
  var theme: AppTheme { snapshot?.theme ?? AppTheme() }
  var artworkMode: String { snapshot?.artworkMode ?? "auto" }
  var canPlay: Bool { player?.key != nil || snapshot?.queue.isEmpty == false }
  func setActive(_ active: Bool) {
    bridge.setVisible(active)
    if ProcessInfo.processInfo.arguments.contains(where: { $0.hasPrefix("--test-profile=") }) {
      UIApplication.shared.isIdleTimerDisabled = active
    }
    if active, let snapshot {
      updateIcon(for: snapshot.theme.name)
    } else {
      iconTask?.cancel()
    }
  }
  func send(_ type: String, _ fields: [String: Any] = [:]) {
    bridge.send(fields.merging(["type": type]) { _, new in new })
  }
  func invoke(_ token: UInt64) { send("invoke", ["token": token]) }
  func search() { open("search") }
  func open(_ sheet: String) {
    send("sheet", ["sheet": sheet])
  }
  func closeSheet() {
    guard let destination = sheet ?? (settings ? "settings" : nil) else { return }
    send("dismiss", ["sheet": destination])
  }
  func selectTab(_ tab: String) {
    if tab == "search" {
      search()
      return
    }
    if tab != "now_playing" { libraryTab = tab }
    self.tab = tab
    send("tab", ["tab": tab])
  }
  func dismissNowPlaying() { selectTab(libraryTab) }
  func setPath(_ next: [String]) {
    if next.count < path.count { send("pop", ["depth": next.count]) }
    path = next
  }
  func setNativePath(_ next: [String], from rendered: [String]) {
    guard !settings, sheet == nil, tab == "library", path == rendered,
      next.count < rendered.count, Array(rendered.prefix(next.count)) == next
    else { return }
    setPath(next)
  }
  func transport(_ operation: String) { mediaSession?.perform(operation) }
  private func apply(_ update: Update) {
    if let error = update.error { self.error = error }
    if let url = update.browserURL {
      UIApplication.shared.open(url) { [weak self] opened in
        if !opened { Task { @MainActor in self?.error = "Could not open the browser." } }
      }
    }
    if let request = update.externalSearch {
      Task { @MainActor in
        let opened = await request.open { url, appOnly in
          await UIApplication.shared.open(url, options: appOnly ? [.universalLinksOnly: true] : [:])
        }
        if !opened { error = "Could not open \(request.label)." }
      }
    }
    if let model = update.model, model != snapshot {
      let sameNavigationRoot = snapshot?.libraryNavigationId == model.libraryNavigationId
      if snapshot?.theme.name != model.theme.name {
        updateIcon(for: model.theme.name)
      }
      snapshot = model
      tab = model.tab
      if model.tab != "now_playing" { libraryTab = model.tab }
      settings = model.settingsOpen
      sheet = model.sheet == "settings" ? nil : model.sheet
      var next = model.columns.map(\.id)
      if model.trackDetail, let detail = model.detail { next.append("track:\(detail.id)") }
      if next != path {
        withAnimation(
          sameNavigationRoot && !UIAccessibility.isReduceMotionEnabled
            ? .easeInOut(duration: 0.22) : nil
        ) {
          path = next
        }
      }
    }
    if let lyrics = update.lyrics { snapshot?.lyricsView = lyrics }
    if let player = update.player, player != self.player {
      self.player = player
      mediaSession?.update(player)
    } else if let position = update.position, player != nil {
      player?.position = position
      mediaSession?.updatePosition(position)
    }
  }
  private func updateIcon(for theme: String) {
    iconTask?.cancel()
    let name = theme == "system" ? nil : "AppIcon-\(theme)"
    iconTask = Task { @MainActor in
      do { try await Task.sleep(for: .milliseconds(500)) } catch { return }
      let app = UIApplication.shared
      guard app.applicationState == .active, app.supportsAlternateIcons,
        app.alternateIconName != name
      else { return }
      do { try await app.setAlternateIconName(name) } catch {
        Logger(subsystem: "local.textamp.ios", category: "appearance")
          .error("Could not update the app icon: \(error.localizedDescription)")
      }
    }
  }
}
