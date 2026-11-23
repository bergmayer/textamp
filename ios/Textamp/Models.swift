import SwiftUI

struct Update: Decodable, Equatable {
  var lyrics: LyricsViewModel? = nil
  var position: Double? = nil
  let model: Snapshot?
  let player: Player?
  let error: String?
  let externalSearch: ExternalSearchRequest?
  let browserURL: URL?
}
struct ExternalSearchRequest: Decodable, Equatable {
  let label: String
  let url: URL
  let webURL: URL
  let requiresApp: Bool

  @MainActor
  func open(using handoff: (URL, Bool) async -> Bool) async -> Bool {
    if requiresApp { return await handoff(webURL, true) }
    if await handoff(url, url.scheme == "https") { return true }
    return await handoff(webURL, false)
  }
}
struct CommandButton: Decodable, Equatable, Identifiable {
  var id: String { controlId ?? String(token) }
  let controlId: String?
  let token: UInt64
  let label: String
  var section: String?
  var menuGroup: String?
  var group: String?
  var subtitle: String?
  var description: String?
  var checked: Bool?
  var active: Bool?
  var disabled: String?
  var value: String?
  var refresh: SimpleCommand?
  var canExpand: Bool?
  var submenu: Bool?
  var destructive: Bool?
  var kind: String?
  var shortcut: String?
  var symbol: String?
  var mark: String?
  var bounds: [Int]?
  var context: [CommandButton]?
  var choices: [CommandButton]?
}
struct SimpleCommand: Decodable, Equatable {
  let token: UInt64
  let label: String
}
struct MusicRow: Decodable, Equatable, Identifiable {
  var pending: String?
  var cancel: CommandButton?
  let id: String
  let key: String
  let index: Int
  let position: Int?
  let title: String
  let subtitle: String
  let kind: String
  let year: Int?
  let artwork: String?
  let selected: Bool
  var batchSelected: Bool?
  let details: SimpleCommand?
  let context: [CommandButton]?
  let activation: UInt64?
  let disclosure: UInt64?
}
extension Array where Element == CommandButton {
  var menuSections: [[CommandButton]] {
    var groups: [String] = []
    var sections: [String: [CommandButton]] = [:]
    for command in self {
      let group = command.menuGroup ?? ""
      if sections[group] == nil { groups.append(group) }
      sections[group, default: []].append(command)
    }
    return groups.compactMap { sections[$0] }
  }
}
struct Column: Decodable, Equatable, Identifiable {
  var playlistEditing: Bool?
  var playlistEditControls: [CommandButton]?
  var summary: String?
  let id: String
  let title: String
  var compactTitle: String?
  let rows: [MusicRow]
  let count: Int
  let more: Bool?
  let sections: [ColumnSection]?
  let selectedPosition: Int?
  let artworkStart: Int?
  let detail: Bool
  let filter: String
  let filterActive: Bool?
  let options: [CommandButton]
  let largeArtwork: Bool
  var albumDetailsArtwork: String?
  var albumArtworkKey: String?
  let albumDetails: CommandButton?
  let playback: [CommandButton]?
  let radio: Bool?
  let sidebar: Bool?
  let stationDetails: StationDetails?
  let stationDescription: String?
  let descriptionPrompt: Bool?
}
struct ColumnSection: Decodable, Equatable {
  let label: String
  let position: Int
}
struct StationDetails: Decodable, Equatable {
  let input: String?
  let title: String
  let text: String
  let actions: [CommandButton]
}
struct ReviewPassage: Decodable, Equatable {
  let text: String
  let italic: Bool
}
struct Details: Decodable, Equatable {
  let review: [ReviewPassage]?
  let albumURL: URL?
  let albumArtist: CommandButton?
  let id: String
  let title: String
  let kind: String
  let subtitle: String
  let rows: [[String]]
  let text: String
  let loading: Bool
  let artwork: String
  let actions: [CommandButton]
  let links: [CommandButton]
  let related: [CommandButton]
  let similar: [CommandButton]
  let relatedTitle: String
  let message: String
}
struct ConnectionStatus: Decodable, Equatable {
  let owner: String
  let provider: String
  let editing: Bool
  let fields: [String]
  let busy: Bool
  let error: String?
}
struct LibrarySettingsPage: Decodable, Equatable {
  let title: String
  let actions: [CommandButton]
}
struct LibraryInfo: Decodable, Equatable, Identifiable {
  let id: String
  let provider: String
  let token: UInt64
  let label: String
  let subtitle: String?
  let checked: Bool?
  let refresh: SimpleCommand?
  let pages: [LibrarySettingsPage]
}
struct InputForm: Decodable, Equatable {
  let title: String
  let text: String
  let submit: String?
}
struct UtilityDialog: Decodable, Equatable {
  let title: String
  let text: String
  let loading: Bool
  let choices: [CommandButton]
  let actions: [CommandButton]
  let query: String?
}
struct JourneyForm: Decodable, Equatable {
  let start: String
  let end: String
  let count: String
  let query: String
  let radio: Bool
  let drilled: Bool
  let ready: Bool
  let loading: Bool
  let choices: [CommandButton]
  let actions: [CommandButton]
  let error: String?
}
struct Snapshot: Decodable, Equatable {
  var discovery: [CommandButton]?
  let externalSearches: [CommandButton]
  var activityAction: UInt64?
  var libraryNavigationId: String?
  let queueMode: String
  let queueModes: [CommandButton]
  let tab: String
  let sheet: String?
  let settingsOpen: Bool
  let libraryLoading: Bool
  let roots: [CommandButton]
  let sidebarGroups: [CommandButton]
  let columns: [Column]
  let detail: Details?
  var canReturn: Bool?
  let trackDetail: Bool
  let queue: [MusicRow]
  let tools: [CommandButton]
  let settings: [CommandButton]
  let libraries: [LibraryInfo]
  let caches: [LibraryCache]
  let search: [CommandButton]
  let searchQuery: String
  let searchPending: Bool
  let palette: [CommandButton]
  let palettePath: [String]
  let paletteBack: SimpleCommand?
  let connection: ConnectionStatus?
  let theme: AppTheme
  let artworkMode: String
  let loading: String?
  let notice: StatusNotice?
  let offline: Bool
  var lyricsView: LyricsViewModel?
  let lyrics: String?
  let downloads: [CommandButton]
  let singlesLimit: Int
  let journey: JourneyForm?
  let input: InputForm?
  let confirmation: InputForm?
  let dialog: UtilityDialog?
}
struct LibraryCache: Decodable, Equatable, Identifiable {
  let id: String
  let label: String
  let metadata: String
  let artwork: String
  let status: String
  let running: Bool
  let actions: [CommandButton]
}
struct StatusNotice: Decodable, Hashable {
  var action: UInt64?
  var actionLabel: String?
  let message: String
  let kind: String
}
struct Player: Decodable, Equatable {
  let key: String?
  let title: String
  let artist: String
  let albumArtist: String?
  let album: String
  var waveform: [Float]?
  let waveformMessage: String?
  let playing: Bool
  let autoMix: Bool?
  var position: Double
  let duration: Double
  let volume: Double
  let artwork: String
  let rows: [[String]]
}
struct AppTheme: Decodable, Equatable {
  var name = "system"
  var background: String?
  var surface: String?
  var foreground: String?
  var secondary: String?
  var accent: String?
  var border: String?
  var errorForeground: String?
  var system: Bool { name == "system" }
  var colorScheme: ColorScheme? {
    guard !system else { return nil }
    guard let background, let rgb = UInt64(background.dropFirst(), radix: 16) else { return .dark }
    let brightness =
      Double((rgb >> 16) & 255) * 0.2126 + Double((rgb >> 8) & 255) * 0.7152 + Double(rgb & 255)
      * 0.0722
    return brightness < 130 ? .dark : .light
  }
  var bg: Color { system ? Color(uiColor: .systemGroupedBackground) : Color(hex: background) }
  var card: Color {
    system ? Color(uiColor: .secondarySystemGroupedBackground) : Color(hex: surface)
  }
  var text: Color { system ? Color(uiColor: .label) : Color(hex: foreground) }
  var muted: Color { system ? Color(uiColor: Self.systemSecondaryInk) : Color(hex: secondary) }
  var tint: Color { system ? Color(uiColor: .systemBlue) : Color(hex: accent) }
  var error: Color { system ? Color(uiColor: .systemRed) : Color(hex: errorForeground) }
  var columnDivider: Color {
    if system { return Color(uiColor: .secondaryLabel) }
    let surfaces = [background, surface].compactMap(Self.luminance)
    for candidate in [border, secondary, foreground] {
      if let light = Self.luminance(candidate),
        surfaces.allSatisfy({ (max(light, $0) + 0.05) / (min(light, $0) + 0.05) >= 3 })
      {
        return Color(hex: candidate)
      }
    }
    return text
  }
  var onTint: Color {
    if system { return Color(uiColor: Self.systemTintInk) }
    guard let light = Self.luminance(accent) else { return .white }
    return light > 0.179 ? .black : .white
  }
  private static let systemTintInk = UIColor { traits in
    var r: CGFloat = 0
    var g: CGFloat = 0
    var b: CGFloat = 0
    let blue = UIColor.systemBlue.resolvedColor(with: traits)
    guard blue.getRed(&r, green: &g, blue: &b, alpha: nil) else { return .white }
    return luminance(Double(r), Double(g), Double(b)) > 0.179 ? .black : .white
  }
  private static func luminance(_ hex: String?) -> Double? {
    guard let hex, let rgb = UInt64(hex.dropFirst(), radix: 16) else { return nil }
    return luminance(
      Double((rgb >> 16) & 255) / 255, Double((rgb >> 8) & 255) / 255, Double(rgb & 255) / 255)
  }
  private static let systemSecondaryInk = UIColor { traits in
    let original = UIColor.secondaryLabel.resolvedColor(with: traits)
    var red: CGFloat = 0
    var green: CGFloat = 0
    var blue: CGFloat = 0
    var alpha: CGFloat = 0
    guard original.getRed(&red, green: &green, blue: &blue, alpha: &alpha) else { return original }
    let backgrounds = [UIColor.systemGroupedBackground, .secondarySystemGroupedBackground]
      .compactMap {
        color -> [Double]? in
        var r: CGFloat = 0
        var g: CGFloat = 0
        var b: CGFloat = 0
        guard color.resolvedColor(with: traits).getRed(&r, green: &g, blue: &b, alpha: nil) else {
          return nil
        }
        return [Double(r), Double(g), Double(b)]
      }
    let ink = [Double(red), Double(green), Double(blue)]
    for step in 0...20 {
      let opacity = Double(alpha) + (1 - Double(alpha)) * Double(step) / 20
      let readable = backgrounds.allSatisfy { background in
        let mixed = zip(ink, background).map { pair in pair.0 * opacity + pair.1 * (1 - opacity) }
        let a = luminance(mixed[0], mixed[1], mixed[2])
        let b = luminance(background[0], background[1], background[2])
        return (max(a, b) + 0.05) / (min(a, b) + 0.05) >= 4.5
      }
      if readable { return original.withAlphaComponent(CGFloat(opacity)) }
    }
    return UIColor.label.resolvedColor(with: traits)
  }
  private static func luminance(_ red: Double, _ green: Double, _ blue: Double) -> Double {
    func linear(_ c: Double) -> Double { c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4) }
    return 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
  }
}
extension Color {
  init(hex: String?) {
    let value = UInt64((hex ?? "#808080").dropFirst(), radix: 16) ?? 0x808080
    self.init(
      red: Double((value >> 16) & 255) / 255, green: Double((value >> 8) & 255) / 255,
      blue: Double(value & 255) / 255)
  }
}
struct ThemeKey: EnvironmentKey { static let defaultValue = AppTheme() }
extension EnvironmentValues {
  var textampTheme: AppTheme {
    get { self[ThemeKey.self] }
    set { self[ThemeKey.self] = newValue }
  }
}

struct LyricsViewModel: Decodable, Equatable {
  let track: String
  let title: String
  let message: String?
  let loading: Bool
  let follow: Bool
  let synced: Bool
  let language: Int
  let languages: [String]
  let active: Int?
  let lines: [LyricsDisplayLine]
}
struct LyricsDisplayLine: Decodable, Equatable {
  let value: String
  let active: Bool
  let spans: [LyricsSpan]
}
struct LyricsSpan: Decodable, Equatable {
  let text: String
  let active: Bool
}
