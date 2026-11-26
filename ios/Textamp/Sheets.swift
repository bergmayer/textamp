import SwiftUI
import UniformTypeIdentifiers

struct ArtistWebSearchButton: View {
  @Environment(AppModel.self) private var model
  let command: CommandButton
  var showsLabel = false
  var body: some View {
    Button { model.invoke(command.token) } label: {
      HStack {
        Image(command.mark ?? "Google").resizable().aspectRatio(contentMode: .fit).frame(width: 24, height: 24)
        if showsLabel { Text(command.label) }
      }.frame(minWidth: 44, minHeight: 44)
    }.accessibilityLabel(command.label)
  }
}

struct SheetDismissButton: View {
  var label = "Done"
  let action: () -> Void
  var body: some View {
    Button(label, systemImage: "chevron.down", action: action)
      .labelStyle(.iconOnly)
      .buttonStyle(.plain)
      .foregroundStyle(.tint)
  }
}

struct SheetDismissToolbarItem: ToolbarContent {
  var label = "Done"
  let action: () -> Void

  var body: some ToolbarContent {
    let item = ToolbarItem(placement: .topBarTrailing) { SheetDismissButton(label: label, action: action) }
    if #available(iOS 26, *) {
      item.sharedBackgroundVisibility(.hidden)
    } else {
      item
    }
  }
}

private struct LibrarySheetDetents: ViewModifier {
  let sheet: String?
  let dismiss: () -> Void
  @State private var lyricsHeight: PresentationDetent = .large

  @ViewBuilder func body(content: Content) -> some View {
    if sheet == "lyrics" {
      content.modifier(PlaybackSheetDetents(height: $lyricsHeight, dismiss: dismiss))
        .onDisappear { lyricsHeight = .large }
    } else {
      content.presentationDetents(sheet == "downloads" ? [.large] : [.medium, .large])
        .presentationDragIndicator(.visible)
    }
  }
}

struct PresentedUtilitySheet: View {
  @Environment(AppModel.self) private var model

  var body: some View {
    if model.sheet == "input", let form = model.snapshot?.input, form.fileMode != nil,
      let generation = form.generation {
      PlaylistFileSheet(form: form, generation: generation)
        .id("\(generation)-\(form.fileMode ?? "")-\(form.text)")
        .presentationDetents([.large])
        .interactiveDismissDisabled()
    } else {
      SearchSheet()
    }
  }
}

private struct PlaylistFileSheet: View {
  @Environment(AppModel.self) private var model
  let form: InputForm
  let generation: UInt64
  @State private var exportURL: URL?
  @State private var temporaryDirectory: URL?

  var body: some View {
    Group {
      if form.fileMode == "import" || exportURL != nil {
        PlaylistDocumentPicker(exportURL: exportURL) { url in
          var fields: [String: Any] = ["generation": generation]
          if let url { fields["path"] = url.path }
          model.send("playlist_file_picked", fields)
        }
      } else {
        ProgressView().task {
          guard let asset = form.asset else { return }
          let data = await withCheckedContinuation { continuation in
            model.bridge.asset(asset) { continuation.resume(returning: $0) }
          }
          guard !Task.isCancelled else { return }
          let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
          do {
            guard let data else { throw CocoaError(.fileReadNoSuchFile) }
            let name = URL(fileURLWithPath: form.text).lastPathComponent
            let file = directory.appendingPathComponent(name)
            try await Task.detached {
              try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
              try data.write(to: file, options: .atomic)
            }.value
            if Task.isCancelled {
              try? FileManager.default.removeItem(at: directory)
              return
            }
            temporaryDirectory = directory
            exportURL = file
          } catch {
            try? FileManager.default.removeItem(at: directory)
            model.closeSheet()
            model.error = error.localizedDescription
          }
        }
      }
    }.onDisappear {
      if let temporaryDirectory { try? FileManager.default.removeItem(at: temporaryDirectory) }
    }
  }
}

private struct PlaylistDocumentPicker: UIViewControllerRepresentable {
  let exportURL: URL?
  let complete: (URL?) -> Void

  func makeCoordinator() -> Coordinator { Coordinator(complete: complete) }
  func makeUIViewController(context: Context) -> UIDocumentPickerViewController {
    let picker: UIDocumentPickerViewController
    if let exportURL {
      picker = UIDocumentPickerViewController(forExporting: [exportURL], asCopy: true)
    } else {
      let types = ["m3u", "m3u8", "csv"].compactMap { UTType(filenameExtension: $0) }
      picker = UIDocumentPickerViewController(forOpeningContentTypes: types, asCopy: true)
    }
    picker.delegate = context.coordinator
    return picker
  }
  func updateUIViewController(_ controller: UIDocumentPickerViewController, context: Context) {}
  final class Coordinator: NSObject, UIDocumentPickerDelegate {
    let complete: (URL?) -> Void
    init(complete: @escaping (URL?) -> Void) { self.complete = complete }
    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) { complete(nil) }
    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
      complete(urls.first)
    }
  }
}

struct SearchSheet: View {
  enum Field { case query, input }
  @Environment(AppModel.self) var model
  @State private var query = ""
  @State private var selection = 0
  @FocusState private var focus: Field?
  var body: some View {
    NavigationStack {
      Group {
        switch model.sheet {
        case "journey": JourneyFormView()
        case "input", "save_playlist": InputSheet(focus: $focus)
        case "confirm":
          Form {
            Text(model.snapshot?.confirmation?.text ?? "").listRowBackground(model.theme.card)
            Button("Confirm", role: .destructive) { model.send("confirm", ["accept": true]) }
              .listRowBackground(model.theme.card)
            Button("Cancel", role: .cancel) { model.send("confirm", ["accept": false]) }
              .listRowBackground(model.theme.card)
          }
        case "library_picker":
          List(model.snapshot?.libraries ?? []) { library in
            Button {
              model.invoke(library.token)
            } label: {
              HStack {
                Text(library.label)
                Spacer()
                if library.checked == true { Image(systemName: "checkmark") }
              }
            }
            .listRowBackground(model.theme.card)
          }
        case "view_options", "artist_radio", "biography", "text":
          UtilitySheet()
        case "downloads":
          List {
            Section { Text(model.snapshot?.lyrics ?? "").listRowBackground(model.theme.card) }
            ForEach(model.snapshot?.downloads ?? []) { command in
              Button(command.label) { model.invoke(command.token) }.listRowBackground(
                model.theme.card)
            }
          }
        case "lyrics": LyricsScreen()
        default:
          ScrollViewReader { proxy in
            List {
              TextField(
                model.sheet == "palette"
                  ? (choosingPlaylist ? "Find a playlist" : "Find a command") : "Search your library",
                text: $query
              ).focused($focus, equals: .query).textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .submitLabel(.search).onSubmit {
                  if model.sheet == "palette" {
                    if let selectedCommand, selectedCommand.disabled == nil {
                      model.invoke(selectedCommand.token)
                    }
                  } else {
                    submitQuery(explicit: true)
                  }
                }
                .onChange(of: query) { _, _ in
                  selection = 0
                  submitQuery()
                }.listRowBackground(model.theme.card)
                .onKeyPress(.upArrow) { navigatePalette(.upArrow) }
                .onKeyPress(.downArrow) { navigatePalette(.downArrow) }
                .onKeyPress(.leftArrow) { query.isEmpty ? navigatePalette(.leftArrow) : .ignored }
                .onKeyPress(.rightArrow) { query.isEmpty ? navigatePalette(.rightArrow) : .ignored }
              if model.sheet == "search", query.isEmpty, !results.isEmpty {
                Text("Recently Chosen").font(.subheadline).foregroundStyle(model.theme.muted)
                  .listRowBackground(model.theme.card)
              }
              ForEach(results) { item in
                Button {
                  model.invoke(item.token)
                } label: {
                  ChoiceLabel(item: item)
                }.listRowBackground(
                  model.sheet == "palette" && selectedCommand?.id == item.id
                    ? model.theme.tint.opacity(0.15) : model.theme.card
                ).disabled(item.disabled != nil).id(item.id).accessibilityIdentifier(
                  item.controlId ?? item.id)
              }
              if !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && results.isEmpty
                && (model.sheet == "palette"
                  || (model.snapshot?.searchQuery == query
                    && model.snapshot?.searchPending == false))
                && model.snapshot?.loading == nil
              {
                Text("No matches").foregroundStyle(model.theme.muted)
              }
            }
            .onChange(of: selection) { _, _ in
              if let selectedCommand { proxy.scrollTo(selectedCommand.id) }
            }
          }
        }
      }.scrollContentBackground(.hidden).background(model.theme.baseFill)
        .safeAreaInset(edge: .bottom, spacing: 0) {
          ActivityStatus()
        }
        .navigationTitle(title).navigationBarTitleDisplayMode(.inline)
        .toolbar {
          if model.sheet == "palette", query.isEmpty, let back = model.snapshot?.paletteBack {
            ToolbarItem(placement: .topBarLeading) {
              Button("Back", systemImage: "chevron.left") { model.invoke(back.token) }
            }
          }
          if model.sheet == "biography", let search = model.snapshot?.dialog?.search {
            ToolbarItem(placement: .topBarLeading) { ArtistWebSearchButton(command: search) }
          }
          SheetDismissToolbarItem { model.closeSheet() }
        }
    }
    .modifier(LibrarySheetDetents(sheet: model.sheet, dismiss: model.closeSheet))
    .defaultFocus($focus, initialFocus)
    .onAppear { if model.sheet == "search" { focus = .query } }
    .onChange(of: model.sheet) { _, value in
      if value == "search" || value == "palette" {
        query = ""
      }
      focus = initialFocus
    }
    .onChange(of: model.snapshot?.palettePath) { _, _ in
      selection = 0
      if choosingPlaylist { query = "" }
    }
  }
  private var choosingPlaylist: Bool { model.snapshot?.palettePath == ["Add to Playlist"] }
  private var selectedCommand: CommandButton? {
    results.indices.contains(selection) ? results[selection] : nil
  }
  private func navigatePalette(_ key: KeyEquivalent) -> KeyPress.Result {
    guard model.sheet == "palette" else { return .ignored }
    switch key {
    case .upArrow: selection = max(0, selection - 1)
    case .downArrow: selection = min(max(0, results.count - 1), selection + 1)
    case .leftArrow:
      if let back = model.snapshot?.paletteBack { model.invoke(back.token) }
    case .rightArrow:
      if let selectedCommand, selectedCommand.submenu == true {
        model.invoke(selectedCommand.token)
      }
    default: return .ignored
    }
    return .handled
  }
  private var initialFocus: Field? {
    switch model.sheet {
    case "search", "palette": .query
    case "input", "save_playlist": .input
    default: nil
    }
  }
  var results: [CommandButton] {
    if model.sheet == "palette" { return model.snapshot?.palette ?? [] }
    guard let snapshot = model.snapshot, snapshot.searchQuery == query else { return [] }
    return snapshot.search
  }
  var title: String {
    switch model.sheet {
    case "journey": return model.snapshot?.journey?.radio == true ? "Radio" : "Journey"
    case "input": return model.snapshot?.input?.title ?? ""
    case "confirm": return model.snapshot?.confirmation?.title ?? ""
    case "palette":
      if choosingPlaylist { return "Add to Playlist" }
      return query.isEmpty
        ? model.snapshot?.palettePath.last ?? "Command Palette" : "Command Palette"
    case "save_playlist": return "Save Playlist"
    case "lyrics": return "Lyrics"
    case "downloads": return "Downloads"
    case "library_picker": return "Switch Library"
    case "view_options", "artist_radio", "biography", "text":
      return model.snapshot?.dialog?.title ?? ""
    default: return "Search"
    }
  }
  func submitQuery(explicit: Bool = false) {
    switch model.sheet {
    case "search": model.send("search", ["query": query, "submit": explicit])
    case "palette": model.send("palette", ["query": query])
    default: break
    }
  }
}

struct InputSheet: View {
  @Environment(AppModel.self) var model
  @State private var text = ""
  @FocusState.Binding var focus: SearchSheet.Field?
  private var playlist: Bool { model.sheet == "save_playlist" }
  var body: some View {
    Form {
      TextField(playlist ? "Playlist Name" : model.snapshot?.input?.title ?? "", text: $text)
        .focused($focus, equals: .input).submitLabel(.done).onSubmit(submit)
        .onChange(of: text) { _, value in model.send("edit_input", ["text": value]) }
        .listRowBackground(model.theme.card)
      Button(playlist ? "Save Playlist" : model.snapshot?.input?.submit ?? "Save", action: submit)
        .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        .listRowBackground(model.theme.card)
        .accessibilityIdentifier("input-submit")
    }.onAppear {
      text = model.snapshot?.input?.text ?? ""
    }
  }
  private func submit() {
    focus = nil
    model.send("submit_input", ["text": text])
  }
}

struct ChoiceLabel: View {
  @Environment(AppModel.self) var model
  let item: CommandButton
  var body: some View {
    HStack {
      VStack(alignment: .leading, spacing: 4) {
        Text(item.label)
        if let subtitle = item.subtitle, !subtitle.isEmpty {
          Text(subtitle).font(.caption).foregroundStyle(model.theme.muted)
        }
      }
      Spacer()
      if item.checked == true { Image(systemName: "checkmark") }
      if item.submenu == true {
        Image(systemName: "chevron.right").font(.footnote.weight(.semibold))
          .foregroundStyle(model.theme.muted)
      }
    }.padding(.vertical, 4)
  }
}

struct UtilitySheet: View {
  @Environment(AppModel.self) var model
  var body: some View {
    if let dialog = model.snapshot?.dialog, let selected = dialog.selected {
      ArtistMixPicker(dialog: dialog, selected: selected)
    } else {
      List {
        if let dialog = model.snapshot?.dialog {
          if !dialog.text.isEmpty {
            Text(dialog.text).textSelection(.enabled).listRowBackground(model.theme.card)
          }
          if dialog.loading { ProgressView().listRowBackground(model.theme.card) }
          ForEach(dialog.actions) { item in
            Button(item.label) { model.invoke(item.token) }.disabled(item.disabled != nil)
              .listRowBackground(model.theme.card)
          }
          ForEach(dialog.choices) { item in
            Button {
              model.invoke(item.token)
            } label: {
              ChoiceLabel(item: item)
            }
            .disabled(item.disabled != nil).listRowBackground(model.theme.card)
          }
        }
      }
    }
  }
}

private struct ArtistMixPicker: View {
  @Environment(AppModel.self) private var model
  let dialog: UtilityDialog
  let selected: [CommandButton]
  @State private var query: String

  init(dialog: UtilityDialog, selected: [CommandButton]) {
    self.dialog = dialog
    self.selected = selected
    _query = State(initialValue: dialog.query ?? "")
  }

  var body: some View {
    VStack(spacing: 0) {
      HStack {
        HStack {
          Image(systemName: "magnifyingglass").foregroundStyle(model.theme.muted)
          TextField("Find artists", text: $query)
            .textInputAutocapitalization(.never).autocorrectionDisabled().submitLabel(.search)
          if !query.isEmpty {
            Button { query = "" } label: { Image(systemName: "xmark.circle.fill") }
              .foregroundStyle(model.theme.muted).accessibilityLabel("Clear search")
          }
        }
        .padding(10).background(model.theme.card, in: RoundedRectangle(cornerRadius: 10))
        if let play = dialog.actions.first {
          Button { model.invoke(play.token) } label: { Label(play.label, systemImage: "play.fill") }
            .buttonStyle(.borderedProminent).fixedSize().disabled(play.disabled != nil)
        }
      }.padding(.horizontal).padding(.vertical, 8)
      if !selected.isEmpty {
        ScrollView(.horizontal) {
          HStack(spacing: 8) {
            ForEach(selected) { artist in
              Button { model.invoke(artist.token) } label: {
                Label { Text(artist.label) } icon: { Image(systemName: "xmark.circle.fill") }
                  .labelStyle(.titleAndIcon)
                  .font(.subheadline).padding(.horizontal, 12).padding(.vertical, 8)
                  .background(model.theme.card, in: Capsule())
              }.buttonStyle(.plain).accessibilityLabel("Remove \(artist.label)")
            }
          }.padding(.horizontal)
        }
        .accessibilityElement(children: .contain).accessibilityLabel("Selected artists")
        .padding(.bottom, 6)
      }
      List {
        if !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && dialog.query == query {
          if dialog.choices.isEmpty {
            Text("No matching artists").foregroundStyle(model.theme.muted)
              .listRowBackground(model.theme.card)
          }
          ForEach(dialog.choices) { artist in
            Button {
              model.invoke(artist.token)
              query = ""
            } label: {
              HStack {
                Text(artist.label)
                Spacer()
                Image(systemName: "plus").accessibilityHidden(true)
              }.contentShape(Rectangle())
            }.disabled(artist.disabled != nil).accessibilityLabel(artist.label)
              .listRowBackground(model.theme.card)
          }
        }
      }.listStyle(.plain).scrollContentBackground(.hidden).scrollDismissesKeyboard(.interactively)
    }
    .background(model.theme.baseFill)
    .onChange(of: query) { _, value in model.send("artist_radio_query", ["query": value]) }
  }
}
private struct AboutLogo: View {
  @Environment(\.textampTheme) private var theme

  private static let artwork: UIImage? = {
    guard let url = Bundle.main.url(forResource: "textamp_clean", withExtension: "ansi"),
      let source = try? String(contentsOf: url, encoding: .utf8)
    else { return nil }

    let font = UIFont.monospacedSystemFont(ofSize: 16, weight: .regular)
    let cellWidth = ("M" as NSString).size(withAttributes: [.font: font]).width
    let cellHeight = cellWidth * 2
    let rows = source.split(separator: "\n").map { line in
      var foreground: CGFloat = 0
      var background: CGFloat = 0
      var cells: [(Character, CGFloat, CGFloat)] = []
      for run in line.split(separator: "\u{1b}") {
        guard run.first == "[", let end = run.firstIndex(where: { $0.isLetter }) else { continue }
        if run[end] == "m" {
          let codes = run[run.index(after: run.startIndex)..<end].split(separator: ";").compactMap { Int($0) }
          var i = 0
          while i < codes.count {
            if codes[i] == 0 { foreground = 0; background = 0 }
            if (codes[i] == 38 || codes[i] == 48), i + 4 < codes.count, codes[i + 1] == 2 {
              let light = CGFloat(54 * codes[i + 2] + 183 * codes[i + 3] + 19 * codes[i + 4]) / (256 * 255)
              if codes[i] == 38 { foreground = light } else { background = light }
              i += 5
            } else { i += 1 }
          }
        }
        cells += run.suffix(from: run.index(after: end)).map { ($0, foreground, background) }
      }
      return cells
    }.filter { !$0.isEmpty }
    guard let width = rows.map(\.count).max(), width > 0 else { return nil }
    return UIGraphicsImageRenderer(size: CGSize(width: cellWidth * CGFloat(width),
                                                height: cellHeight * CGFloat(rows.count))).image { renderer in
      for (row, cells) in rows.enumerated() {
        for (column, cell) in cells.enumerated() {
          let rect = CGRect(x: CGFloat(column) * cellWidth, y: CGFloat(row) * cellHeight,
                            width: cellWidth, height: cellHeight)
          UIColor(white: cell.2, alpha: 1).setFill()
          renderer.fill(rect)
          renderer.cgContext.saveGState()
          renderer.cgContext.clip(to: rect)
          (String(cell.0) as NSString).draw(at: rect.origin, withAttributes: [
            .font: font, .foregroundColor: UIColor(white: cell.1, alpha: 1)
          ])
          renderer.cgContext.restoreGState()
        }
      }
    }
  }()

  var body: some View {
    if let artwork = Self.artwork {
      theme.tint.aspectRatio(artwork.size.width / artwork.size.height, contentMode: .fit)
        .mask(Image(uiImage: artwork).resizable().luminanceToAlpha())
        .accessibilityElement(children: .ignore).accessibilityLabel("Textamp")
        .accessibilityAddTraits(.isImage).accessibilityIdentifier("about-logo")
    } else { Text("Textamp").font(.headline) }
  }
}

struct SettingsScreen: View {
  private enum Route: Hashable {
    case library(String)
    case connection(String)
    case preferences(String)
    case caches
    case activityLog
  }
  @Environment(AppModel.self) var model
  @State private var path: [Route] = []
  @State private var activity = ""
  var body: some View {
    VStack(spacing: 0) {
      NavigationStack(path: $path) {
        List {
          Section("Libraries") {
            ForEach(model.snapshot?.libraries ?? []) { library in
              NavigationLink(value: Route.library(library.id)) {
                HStack {
                  VStack(alignment: .leading) {
                    Text(library.label)
                    Text(library.subtitle ?? "").font(.caption).foregroundStyle(model.theme.muted)
                  }
                  Spacer()
                  if library.checked == true { Image(systemName: "checkmark") }
                }
              }
              .contextMenu {
                if let refresh = library.refresh {
                  Button(refresh.label) { model.invoke(refresh.token) }
                }
              }
            }
            NavigationLink("Add Navidrome Library", value: Route.connection("navidrome"))
            NavigationLink("Add WebDAV Library", value: Route.connection("webdav"))
          }.listRowBackground(model.theme.card)
          ForEach(sections, id: \.self) { section in
            NavigationLink(section, value: Route.preferences(section))
              .listRowBackground(model.theme.card)
              .accessibilityIdentifier("settings-section-\(section)")
          }
          NavigationLink("Caches", value: Route.caches).listRowBackground(model.theme.card)
          Section("About") {
            AboutLogo().frame(maxWidth: 480).padding(.vertical, 8)
              .contentShape(Rectangle()).onTapGesture(count: 4) { path.append(.activityLog) }
          }.listRowBackground(model.theme.card)
        }.scrollContentBackground(.hidden).background(model.theme.baseFill)
          .navigationDestination(for: Route.self) { route in
            switch route {
            case .library(let id): LibrarySettingsScreen(libraryID: id)
            case .connection(let provider): ConnectionForm(provider: provider)
            case .preferences(let section): PreferencesScreen(section: section)
            case .caches: CacheScreen()
            case .activityLog:
              ScrollView {
                Text(activity).font(.callout).textSelection(.enabled)
                  .frame(maxWidth: .infinity, alignment: .leading).padding()
              }.navigationTitle("Activity Log").task { activity = await model.bridge.activityHistory() }
            }
          }
          .safeAreaInset(edge: .bottom, spacing: 0) { ActivityStatus() }
          .navigationTitle("Settings").toolbar {
            SheetDismissToolbarItem { model.closeSheet() }
          }
      }
      MainBottomBar()
    }
    .preferredColorScheme(model.theme.colorScheme)
    .sheet(
      isPresented: Binding(
        get: { model.sheet != nil }, set: { if !$0 { model.utilitySheetDismissed() } })
    ) {
      PresentedUtilitySheet().environment(model).environment(\.textampTheme, model.theme)
    }
  }
  private var sections: [String] {
    var seen = Set<String>()
    return (model.snapshot?.settings ?? []).compactMap { item in
      guard let source = item.section else { return nil }
      let section = ["Search", "Audio"].contains(source) ? "Behavior" : source
      guard seen.insert(section).inserted else { return nil }
      return section
    }
  }
}
struct LibrarySettingsScreen: View {
  @Environment(AppModel.self) var model
  let libraryID: String
  private var library: LibraryInfo? { model.snapshot?.libraries.first { $0.id == libraryID } }
  var body: some View {
    Form {
      if let library {
        ForEach(library.pages, id: \.title) { page in
          Section(page.title) {
            ForEach(page.actions) { action in
              if action.kind == "connection" || action.kind == "audiomuse" {
                NavigationLink(action.label) {
                  ConnectionForm(
                    provider: action.kind == "audiomuse" ? "audiomuse" : library.provider,
                    editToken: action.token)
                }
              } else if action.kind == "toggle" {
                Toggle(
                  action.label,
                  isOn: Binding(
                    get: { action.checked == true },
                    set: { _ in model.invoke(action.token) }
                  )
                ).toggleStyle(.switch)
              } else {
                Button(action.label, role: action.destructive == true ? .destructive : nil) {
                  model.invoke(action.token)
                }
                .disabled(action.disabled != nil)
              }
            }
          }.listRowBackground(model.theme.card)
        }
      }
    }.scrollContentBackground(.hidden).background(model.theme.baseFill)
      .navigationTitle(library?.label ?? "Library").navigationBarTitleDisplayMode(.inline)
  }
}

struct CacheScreen: View {
  @Environment(AppModel.self) var model
  var body: some View {
    Form {
      ForEach(model.snapshot?.caches ?? []) { cache in
        Section(cache.label) {
          LabeledContent("Metadata and analysis", value: cache.metadata)
          LabeledContent("Downloaded artwork", value: cache.artwork)
          HStack {
            if cache.running { ProgressView().controlSize(.small) }
            Text(cache.status).font(.footnote).foregroundStyle(model.theme.muted)
          }
          ForEach(cache.actions) { action in
            Button(action.label, role: action.destructive == true ? .destructive : nil) {
              model.invoke(action.token)
            }.accessibilityIdentifier(action.id)
          }
        }.listRowBackground(model.theme.card)
      }
    }.scrollContentBackground(.hidden).background(model.theme.baseFill)
      .navigationTitle("Caches").navigationBarTitleDisplayMode(.inline)

      .refreshable { model.send("refresh_cache_sizes") }
      .onAppear { model.send("refresh_cache_sizes") }
  }
}
struct PreferencesScreen: View {
  @Environment(AppModel.self) var model
  let section: String
  var preferences: [CommandButton] {
    model.snapshot?.settings.filter {
      section == "Behavior"
        ? ["Search", "Audio"].contains($0.section ?? "") : $0.section == section
    } ?? []
  }
  var groups: [String] {
    var seen = Set<String>()
    return preferences.compactMap {
      let value = group(for: $0)
      return seen.insert(value).inserted ? value : nil
    }.sorted { left, right in
      if left == "Artwork" { return right != "Artwork" }
      if right == "Artwork" { return false }
      return false
    }
  }
  private func group(for item: CommandButton) -> String {
    item.group ?? item.section ?? ""
  }
  var body: some View {
    Form {
      ForEach(groups, id: \.self) { group in
        let items = preferences.filter { self.group(for: $0) == group }
        let exclusive = !items.isEmpty && items.allSatisfy({ $0.kind == "choice" })
        Section {
          if exclusive {
            preferencePicker(group, choices: items)
          } else {
            ForEach(items) { item in
              if item.kind == "toggle" {
                Toggle(isOn: Binding(
                  get: { item.checked == true },
                  set: { _ in model.invoke(item.token) }
                )) {
                  Text(item.label).frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle()).onTapGesture { model.invoke(item.token) }
                }.toggleStyle(.switch).listRowBackground(model.theme.card)
                  .accessibilityIdentifier("preference-\(item.label)")
              } else if let choices = item.choices {
                preferencePicker(item.label, choices: choices, current: item.value)
              } else if let bounds = item.bounds, bounds.count == 3 {
                Stepper(
                  value: Binding(
                    get: { bounds[0] }, set: { model.send("set_singles", ["count": $0]) }
                  ), in: bounds[1]...bounds[2]
                ) {
                  LabeledContent(item.label, value: String(bounds[0]))
                }.listRowBackground(model.theme.card)
              } else {
                Button {
                  model.invoke(item.token)
                } label: {
                  Text(item.label)
                }.listRowBackground(model.theme.card)
              }
            }
          }
        } header: {
          if !exclusive { Text(group) }
        }
      }
    }
    .scrollContentBackground(.hidden).background(model.theme.baseFill).navigationTitle(section)
    .navigationBarTitleDisplayMode(.inline)

  }
  @ViewBuilder
  private func preferencePicker(_ label: String, choices: [CommandButton], current: String? = nil)
    -> some View
  {
    let picker = Picker(
      label,
      selection: Binding(
        get: { choices.first(where: { $0.checked == true })?.token ?? 0 },
        set: { if $0 != 0 { model.invoke($0) } }
      )
    ) {
      ForEach(choices) { choice in
        Text(choice.label).foregroundStyle(model.theme.text).tag(choice.token)
      }
      if let current, !choices.contains(where: { $0.checked == true }) {
        Text(current).tag(UInt64(0)).disabled(true)
      }
    }.listRowBackground(model.theme.card)
      .accessibilityIdentifier(label == "Artwork" ? "artwork-mode" : "preference-choice-\(label)")
    if label == "Theme" || label == "Artwork" {
      picker.pickerStyle(.menu)
    } else {
      picker.pickerStyle(.navigationLink)
    }
  }
}
struct InlineError: View {
  @Environment(AppModel.self) private var model
  let message: String
  var body: some View {
    Label {
      Text(message).foregroundStyle(model.theme.text)
    } icon: {
      Image(systemName: "exclamationmark.circle").foregroundStyle(model.theme.error)
    }
  }
}

struct ConnectionForm: View {
  @Environment(AppModel.self) var model
  let provider: String
  var editToken: UInt64? = nil
  @State private var owner: String?
  @State private var initialized = false
  @State private var fields = Array(repeating: "", count: 8)
  @State private var submitted = false
  @Environment(\.dismiss) var dismiss
  var body: some View {
    Form {
      Section {
        TextField("Server URL", text: $fields[0]).keyboardType(.URL).textContentType(.URL)
        TextField("Username", text: $fields[1]).textContentType(.username)
        SecureField("Password", text: $fields[2]).textContentType(.password)
        TextField(
          provider == "audiomuse" ? "Music-server name" : "Name (optional)", text: $fields[3])
      }.textInputAutocapitalization(.never).autocorrectionDisabled().listRowBackground(
        model.theme.card)
      if provider == "navidrome" {
        Section("AudioMuse (optional)") {
          TextField("AudioMuse URL", text: $fields[4]).keyboardType(.URL)
          TextField("Username", text: $fields[5]).accessibilityIdentifier("audiomuse-username")
          SecureField("Password", text: $fields[6]).accessibilityIdentifier("audiomuse-password")
          TextField("Music-server name", text: $fields[7])
        }.textInputAutocapitalization(.never).autocorrectionDisabled().listRowBackground(
          model.theme.card)
      }
      if let error = model.snapshot?.connection?.error {
        InlineError(message: error)
      }
      Button {
        submitted = true
        var request: [String: Any] = [
          "provider": provider, "fields": Array(fields.prefix(provider == "navidrome" ? 8 : 4)),
        ]
        if let owner { request["owner"] = owner }
        model.send("connect", request)
      } label: {
        HStack {
          Text(editToken == nil ? "Connect" : "Save")
          if model.snapshot?.connection?.busy == true {
            Spacer()
            ProgressView()
          }
        }
      }.disabled(
        fields[0].isEmpty || model.snapshot?.connection?.busy == true
          || (editToken != nil && !initialized))
    }.scrollContentBackground(.hidden).background(model.theme.baseFill).navigationTitle(
      provider == "navidrome" ? "Navidrome" : (provider == "audiomuse" ? "AudioMuse" : "WebDAV")
    )

    .onAppear {
      if let editToken {
        model.invoke(editToken)
      } else {
        fields[7] = "Navidrome"
        initialized = true
      }
    }
    .onChange(of: model.snapshot?.connection) { _, value in
      if let value {
        owner = value.owner
        if editToken != nil && !initialized {
          fields =
            Array(value.fields.prefix(8))
            + Array(repeating: "", count: max(0, 8 - value.fields.count))
          initialized = true
        }
        if value.error != nil { submitted = false }
      } else if submitted {
        dismiss()
      }
    }
    .onDisappear {
      if let connection = model.snapshot?.connection, connection.owner == owner {
        model.send("cancel_connection", ["owner": connection.owner])
      }
    }
  }
}

struct JourneyFormView: View {
  private enum Endpoint: String, Identifiable {
    case start, end
    var id: String { rawValue }
    var label: String { self == .start ? "Start track" : "End track" }
  }
  @Environment(AppModel.self) var model
  @State private var picker: Endpoint?
  @State private var count = "20"
  var body: some View {
    Form {
      if let journey = model.snapshot?.journey {
        Section {
          endpoint(.start, track: journey.start)
          if !journey.radio {
            endpoint(.end, track: journey.end)
            TextField("Maximum tracks (5–100)", text: $count).keyboardType(.numberPad)
              .onChange(of: count) { _, value in model.send("journey_count", ["count": value]) }
          }
        }.listRowBackground(model.theme.card)
        if journey.loading { ProgressView().listRowBackground(model.theme.card) }
        if let error = journey.error { InlineError(message: error) }
        Section {
          ForEach(journey.actions.filter { $0.label != "Back" }) { action in
            Button(action.label) { model.invoke(action.token) }
              .disabled(journey.loading || (!journey.ready && journey.start.isEmpty))
          }
        }.listRowBackground(model.theme.card)
      }
    }
    .onAppear { count = model.snapshot?.journey?.count ?? "20" }
    .sheet(item: $picker) { endpoint in
      JourneyTrackPicker(endpoint: endpoint.rawValue, title: endpoint.label)
        .environment(model).environment(\.textampTheme, model.theme)
        .presentationDetents([.medium, .large]).presentationDragIndicator(.visible)
    }
  }
  private func endpoint(_ endpoint: Endpoint, track: String) -> some View {
    Button {
      picker = endpoint
    } label: {
      HStack {
        VStack(alignment: .leading, spacing: 5) {
          Text(endpoint.label).font(.caption).foregroundStyle(model.theme.muted)
          Text(track.isEmpty ? "Choose a track" : track).foregroundStyle(model.theme.text)
        }
        Spacer()
        Image(systemName: "magnifyingglass").foregroundStyle(model.theme.tint)
      }.frame(minHeight: 44).contentShape(Rectangle())
    }.accessibilityLabel(endpoint.label).accessibilityValue(track)
  }
}

private struct JourneyTrackPicker: View {
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  let endpoint: String
  let title: String
  @State private var query = ""
  @FocusState private var searching: Bool
  var body: some View {
    NavigationStack {
      List {
        TextField("Find a track", text: $query).focused($searching)
          .textInputAutocapitalization(.never).autocorrectionDisabled().submitLabel(.search)
          .onChange(of: query) { _, value in
            model.send("journey_query", ["endpoint": endpoint, "query": value])
          }.listRowBackground(model.theme.card)
        if model.snapshot?.journey?.loading == true { ProgressView() }
        ForEach(model.snapshot?.journey?.choices ?? []) { choice in
          Button {
            model.invoke(choice.token)
            if choice.kind == "track" { dismiss() }
          } label: {
            ChoiceLabel(item: choice)
          }.listRowBackground(model.theme.card)
        }
      }.scrollContentBackground(.hidden).background(model.theme.baseFill)
        .navigationTitle(title).navigationBarTitleDisplayMode(.inline)
        .toolbar {
          if model.snapshot?.journey?.drilled == true {
            ToolbarItem(placement: .topBarLeading) {
              Button("Back", systemImage: "chevron.left") { model.send("journey_back") }
            }
          }
          SheetDismissToolbarItem { dismiss() }
        }
    }.onAppear { model.send("journey_endpoint", ["endpoint": endpoint]) }
      .defaultFocus($searching, true)
  }
}
