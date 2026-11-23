import SwiftUI

final class ArtworkCache: @unchecked Sendable {
  static let shared = ArtworkCache()
  let images = NSCache<NSString, UIImage>()
  let preparation = OperationQueue()
  init() {
    preparation.name = "textamp.artwork"
    preparation.maxConcurrentOperationCount = 2
    preparation.qualityOfService = .userInitiated
    images.totalCostLimit = 40 * 1024 * 1024
    images.countLimit = 256
  }
  func cachedImage(id: String?, mode: String) -> UIImage? {
    guard let id else { return nil }
    return images.object(forKey: "\(id):\(mode)" as NSString)
  }
  func image(id: String, mode: String, bridge: CoreBridge) async -> UIImage? {
    let key = "\(id):\(mode)"
    if let cached = cachedImage(id: id, mode: mode) { return cached }
    return await withCheckedContinuation { continuation in
      bridge.asset(id) { data in
        self.preparation.addOperation {
          let result = data.flatMap {
            mode == "auto"
              ? UIImage(data: $0)?.preparingForDisplay() : CharacterArtwork.render($0, mode: mode)
          }
          if let result {
            self.images.setObject(
              result, forKey: key as NSString,
              cost: result.cgImage.map { $0.bytesPerRow * $0.height } ?? 0)
          }
          continuation.resume(returning: result)
        }
      }
    }
  }

}
struct Artwork: View {
  @Environment(AppModel.self) var model
  let id: String?
  var mode: String? = nil
  @State private var image: UIImage?
  var body: some View {
    Group {
      if let image = ArtworkCache.shared.cachedImage(id: id, mode: effectiveMode) ?? image {
        Image(uiImage: image)
          .renderingMode(isTemplate ? .template : .original)
          .resizable().interpolation(effectiveMode == "auto" ? .high : .none)
          .scaledToFit()
          .foregroundStyle(model.theme.muted)
          .background(isTemplate ? model.theme.card : .clear)
      } else {
        GeometryReader { geometry in
          Image(systemName: "music.note").resizable().scaledToFit()
            .padding(min(geometry.size.width, geometry.size.height) * 0.22)
            .foregroundStyle(model.theme.muted).frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(model.theme.card)
        }
      }
    }
    .accessibilityHidden(true)
    .task(id: "\(id ?? ""):\(effectiveMode)") {
      image = nil
      guard let id else { return }
      let rendered = await ArtworkCache.shared.image(
        id: id, mode: effectiveMode, bridge: model.bridge)
      if !Task.isCancelled { image = rendered }
    }
  }
  private var effectiveMode: String { mode ?? model.artworkMode }
  private var isTemplate: Bool { effectiveMode == "auto" && id?.hasPrefix("placeholder-") == true }
}
private enum CharacterArtwork {
  static func render(_ source: Data, mode: String) -> UIImage? {
    let cells = source.withUnsafeBytes { raw in
      mode.withCString {
        textamp_character_art(raw.bindMemory(to: UInt8.self).baseAddress, raw.count, $0)
      }
    }
    defer { textamp_bytes_free(cells) }
    guard let bytes = cells.data else { return nil }
    let data = Data(bytes: bytes, count: cells.length)
    guard let document = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
      let width = document["width"] as? Int, let height = document["height"] as? Int,
      let cells = document["cells"] as? [[Any]], width > 0, height > 0
    else { return nil }
    let format = UIGraphicsImageRendererFormat()
    format.scale = 1
    format.opaque = true
    return UIGraphicsImageRenderer(
      size: CGSize(width: width * 10, height: height * 20), format: format
    ).image { context in
      for (index, cell) in cells.enumerated() {
        guard cell.count == 3, let symbol = cell[0] as? String, let fg = cell[1] as? [Int],
          let bg = cell[2] as? [Int], fg.count == 3, bg.count == 3
        else { continue }
        let rect = CGRect(x: (index % width) * 10, y: (index / width) * 20, width: 10, height: 20)
        let color: ([Int]) -> UIColor = {
          UIColor(
            red: Double($0[0]) / 255, green: Double($0[1]) / 255, blue: Double($0[2]) / 255,
            alpha: 1)
        }
        color(bg).setFill()
        context.fill(rect)
        if symbol == "▀" {
          color(fg).setFill()
          context.fill(CGRect(x: rect.minX, y: rect.minY, width: 10, height: 10))
        } else {
          (symbol as NSString).draw(
            in: rect,
            withAttributes: [
              .font: UIFont.monospacedSystemFont(ofSize: 17, weight: .regular),
              .foregroundColor: color(fg),
            ])
        }
      }
    }
  }
}
