import AVFoundation
import Foundation
import OSLog
import UIKit

/// Every FFI call is serialized; shutdown cannot race a borrowed runtime handle.
final class CoreBridge: @unchecked Sendable {
  private let queue = DispatchQueue(label: "local.textamp.bridge", qos: .userInitiated)
  private var client: OpaquePointer?
  private var timer: DispatchSourceTimer?
  private let decoder = JSONDecoder()
  var onUpdate: (@Sendable (Update) -> Void)?
  var onError: (@Sendable (String) -> Void)?

  func start() {
    queue.async { [self] in
      do { try configurePaths() } catch {
        onError?(error.localizedDescription)
        return
      }
      var error: UnsafeMutablePointer<CChar>?
      let options =
        ProcessInfo.processInfo.arguments.contains("--silent") ? "{\"silent\":true}" : "{}"
      client = options.withCString { options in
        textamp_start(options, &error) { active in
          let session = AVAudioSession.sharedInstance()
          do {
            if active { try session.setCategory(.playback, mode: .default) }
            try session.setActive(active, options: active ? [] : .notifyOthersOnDeactivation)
            return true
          } catch {
            Logger(subsystem: "local.textamp.ios", category: "audio").error("Audio session activation=\(active) failed: \(error.localizedDescription, privacy: .public)")
            return false
          }
        }
      }
      if let error {
        onError?(String(cString: error))
        textamp_string_free(error)
        return
      }
      let timer = DispatchSource.makeTimerSource(queue: queue)
      timer.schedule(deadline: .now(), repeating: .milliseconds(80))
      timer.setEventHandler { [weak self] in self?.poll() }
      self.timer = timer
      timer.resume()
    }
  }
  func send(_ command: [String: Any]) {
    guard let data = try? JSONSerialization.data(withJSONObject: command),
      let json = String(data: data, encoding: .utf8)
    else { return }
    queue.async { [self] in
      guard let client else { return }
      if let error = json.withCString({ textamp_send(client, $0) }) {
        onError?(String(cString: error))
        textamp_string_free(error)
      }
    }
  }
  func setVisible(_ visible: Bool) {
    queue.async { [self] in
      timer?.schedule(
        deadline: .now(), repeating: visible ? .milliseconds(80) : .seconds(2),
        leeway: .milliseconds(20))
    }
  }
  func activityHistory() async -> String {
    await withCheckedContinuation { continuation in
      queue.async { [self] in
        let data = copy(textamp_activity_history()) ?? Data()
        continuation.resume(returning: String(decoding: data, as: UTF8.self))
      }
    }
  }
  func recordError(_ message: String) {
    queue.async { message.withCString { textamp_record_error($0) } }
  }
  func asset(_ id: String, completion: @escaping @Sendable (Data?) -> Void) {
    queue.async { [self] in
      guard let client else {
        completion(nil)
        return
      }
      let bytes = id.withCString { textamp_asset(client, $0) }
      completion(copy(bytes))
    }
  }
  private func poll() {
    guard let client, let data = copy(textamp_drain(client)) else { return }
    do {
      let update = try decoder.decode(Update.self, from: data)
      if update.model != nil || update.player != nil || update.position != nil || update.lyrics != nil
        || update.error != nil
        || update.externalSearch != nil || update.browserURL != nil
      {
        onUpdate?(update)
      }
    } catch { onError?("Could not read the library view: \(error.localizedDescription)") }
  }
  private func copy(_ bytes: TextampBytes) -> Data? {
    defer { textamp_bytes_free(bytes) }
    guard let data = bytes.data else { return nil }
    return Data(bytes: data, count: bytes.length)
  }
  private func configurePaths() throws {
    var support = try FileManager.default.url(
      for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
    var cache = try FileManager.default.url(
      for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
    if let argument = ProcessInfo.processInfo.arguments.first(where: {
      $0.hasPrefix("--test-profile=")
    }) {
      let name = String(argument.dropFirst("--test-profile=".count))
      guard !name.isEmpty,
        name.utf8.allSatisfy({
          (48...57).contains($0) || (65...90).contains($0)
            || (97...122).contains($0) || $0 == 45
        })
      else { throw CocoaError(.fileWriteInvalidFileName) }
      support = support.appendingPathComponent("Validation/\(name)")
      cache = cache.appendingPathComponent("Validation/\(name)")
    } else {
      // Validation profiles are disposable and separate from the normal library.
      for base in [support, cache] {
        let validation = base.appendingPathComponent("Validation", isDirectory: true)
        if FileManager.default.fileExists(atPath: validation.path) {
          do { try FileManager.default.removeItem(at: validation) } catch {
            Logger(subsystem: "local.textamp.ios", category: "validation")
              .error("Could not discard validation profile: \(error.localizedDescription)")
          }
        }
      }
    }
    for (key, url) in [
      ("XDG_CONFIG_HOME", support.appendingPathComponent("Config")),
      ("XDG_DATA_HOME", support.appendingPathComponent("Data")),
      ("XDG_STATE_HOME", support.appendingPathComponent("State")), ("XDG_CACHE_HOME", cache),
    ] {
      try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
      setenv(key, url.path, 1)
    }
  }
  deinit {
    timer?.cancel()
    if let client { textamp_stop(client) }
  }
}
