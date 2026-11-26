import SwiftUI
import UIKit

@main
struct TextampApp: App {
  @UIApplicationDelegateAdaptor(HomeActionDelegate.self) private var appDelegate
  @State private var model = AppModel()
  @Environment(\.scenePhase) private var scenePhase
  var body: some Scene {
    WindowGroup {
      ContentView().environment(model).environment(\.textampTheme, model.theme)
        .tint(model.theme.tint).foregroundStyle(model.theme.text)
        .background(KeyboardSwipeDismissal())
        .background(WindowAppearance(theme: model.theme))
        .preferredColorScheme(model.theme.colorScheme)
        .onChange(of: HomeActions.shared.pending, initial: true) { _, action in
          if let action {
            HomeActions.shared.pending = nil
            switch action {
            case .search: model.search()
            case .randomAlbumRadio: model.send("home_action", ["action": action.rawValue])
            }
          }
        }
        .onChange(of: scenePhase, initial: true) { _, phase in
          model.setActive(phase == .active)
        }
    }
    .commands {
      CommandMenu("Discover") {
        ForEach(model.snapshot?.discovery ?? []) { command in
          let action = Button(command.label) { model.invoke(command.token) }.disabled(command.disabled != nil)
          if let key = command.shortcut?.first {
            action.keyboardShortcut(KeyEquivalent(Character(key.lowercased())),
              modifiers: key.isUppercase ? [.command, .shift] : [.command, .option])
          } else { action }
        }
        Divider()
        if let search = model.snapshot?.externalSearches.first(where: { $0.controlId == "external-search:AppleMusic" }) {
          Button(search.label) { model.invoke(search.token) }
            .keyboardShortcut("a", modifiers: [.command, .shift])
            .disabled(search.disabled != nil)
        }
      }
    }
  }
}

/// Install on the scene window so sheets, navigation bars and empty result areas
/// all receive the same gesture, including views outside a scroll container.
private struct KeyboardSwipeDismissal: UIViewRepresentable {
  final class Host: UIView, UIGestureRecognizerDelegate {
    private lazy var pan: UIPanGestureRecognizer = {
      let gesture = UIPanGestureRecognizer(target: self, action: #selector(dragged))
      gesture.maximumNumberOfTouches = 1
      gesture.cancelsTouchesInView = false
      gesture.delaysTouchesEnded = false
      gesture.delegate = self
      gesture.isEnabled = false
      return gesture
    }()

    override func didMoveToWindow() {
      super.didMoveToWindow()
      pan.view?.removeGestureRecognizer(pan)
      NotificationCenter.default.removeObserver(self)
      guard let window else { return }
      window.addGestureRecognizer(pan)
      let notifications = NotificationCenter.default
      notifications.addObserver(self, selector: #selector(keyboardShown),
        name: UIResponder.keyboardWillShowNotification, object: nil)
      notifications.addObserver(self, selector: #selector(keyboardHidden),
        name: UIResponder.keyboardDidHideNotification, object: nil)
    }

    @objc private func keyboardShown() { pan.isEnabled = true }
    @objc private func keyboardHidden() { pan.isEnabled = false }

    override func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
      let velocity = pan.velocity(in: window)
      return velocity.y > abs(velocity.x)
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer,
      shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer) -> Bool { true }

    @objc private func dragged() {
      let offset = pan.translation(in: window)
      if (pan.state == .changed || pan.state == .ended), offset.y >= 36, offset.y > abs(offset.x) {
        window?.endEditing(false)
      }
    }
  }

  func makeUIView(context: Context) -> Host {
    let host = Host()
    host.isUserInteractionEnabled = false
    return host
  }

  func updateUIView(_ uiView: Host, context: Context) {}
}

extension EnvironmentValues {
  @Entry fileprivate var queueColumn: Binding<Bool>? = nil
}

struct ContentView: View {
  @Environment(AppModel.self) var model
  @Environment(\.horizontalSizeClass) private var sizeClass
  @State private var transportLayout = PlayerLayout()
  @State private var transportAnimating = false
  @GestureState private var transportLift: CGFloat = 0
  @SceneStorage("tablet.library.visible") private var libraryVisible = true
  @SceneStorage("tablet.queue.visible") private var queueVisible = true

  var body: some View {
    GeometryReader { geometry in
      let wide =
        UIDevice.current.userInterfaceIdiom == .pad && sizeClass == .regular
        && geometry.size.width >= 820
      workspace(wide: wide, width: geometry.size.width)
        .environment(\.queueColumn, wide ? $queueVisible : nil)
        .onChange(of: wide && model.tab == "queue", initial: true) { _, requested in
          if requested {
            queueVisible = true
            model.selectTab("library")
          }
        }
    }
  }

  private func workspace(wide: Bool, width: CGFloat) -> some View {
    VStack(spacing: 0) {
      let rim: CGFloat = model.theme.isBrushedAluminum ? (wide ? 4 : 2) : 0
      workspacePanels(wide: wide, width: width - 2 * rim)
        .overlay {
          if model.theme.isBrushedAluminum {
            Rectangle().strokeBorder(model.theme.columnDivider, lineWidth: 1)
              .allowsHitTesting(false).accessibilityHidden(true)
          }
        }
        .padding(.horizontal, rim)
        .padding(.vertical, rim / 2)
      if model.settings || model.sheet != nil {
        Color.clear.frame(height: 54)
      } else {
        MainBottomBar()
      }
    }
    .background { Rectangle().fill(model.theme.baseFill).ignoresSafeArea() }
    .onChange(of: model.libraryTab) { _, _ in
      UIApplication.shared.sendAction(
        #selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
    }
    .sheet(isPresented: Binding(
      get: { !model.settings && model.sheet != nil },
      set: { if !$0 && !model.settings { model.utilitySheetDismissed() } }
    )) {
      PresentedUtilitySheet().environment(model).environment(\.textampTheme, model.theme)
    }
    .fullScreenCover(
      isPresented: Binding(
        get: { model.settings },
        set: { if !$0 && model.settings { model.closeSheet() } })
    ) {
      SettingsScreen().environment(model).environment(\.textampTheme, model.theme)
    }
    .alert(
      "Textamp",
      isPresented: Binding(get: { model.error != nil }, set: { if !$0 { model.error = nil } })
    ) {
      Button("OK", role: .cancel) { model.error = nil }
    } message: {
      Text(model.error ?? "")
    }
  }

  private func workspacePanels(wide: Bool, width: CGFloat) -> some View {
    let expanded = !wide && model.tab == "now_playing"
    return HStack(spacing: 0) {
      VStack(spacing: 0) {
        let showingLibrary = wide ? model.tab != "now_playing" : model.libraryTab == "library"
        ZStack {
          Group {
            if wide {
              TabletLibrary(
                rootWidth: min(242, max(215, width * 0.18)), sidebarVisible: $libraryVisible)
            } else {
              LibraryBrowser()
            }
          }
          .opacity(showingLibrary ? 1 : 0)
          .allowsHitTesting(showingLibrary)
          .accessibilityHidden(!showingLibrary)
          if !showingLibrary {
            if wide {
              NowPlayingNavigation(inWorkspace: true)
            } else {
              NavigationStack { QueueScreen() }
            }
          }
        }
        if wide && model.tab != "now_playing" { ActivityStatus() }
        if !wide || model.tab != "now_playing" {
          MiniPlayer()
            .opacity(!wide && transportAnimating ? 0 : 1)
            .contentShape(Rectangle())
            .simultaneousGesture(
              DragGesture(minimumDistance: 12, coordinateSpace: .global)
                .updating($transportLift) { value, lift, _ in
                  let up = -value.translation.height
                  if !wide && (lift > 0 || up > abs(value.translation.width)) {
                    lift = max(0, up)
                  }
                }
                .onEnded { value in
                  let up = -value.translation.height
                  if up > 30 && up > abs(value.translation.width) {
                    model.selectTab("now_playing")
                  }
                })
        }
        if !wide && !expanded { ActivityStatus() }
      }
      if wide && queueVisible {
        TabletColumnDivider()
        NavigationStack { QueueScreen(docked: true) }
          .frame(width: min(320, max(306, width * 0.23)))
      }
    }
    .allowsHitTesting(!expanded)
    .accessibilityHidden(expanded)
    .environment(transportLayout)
    .overlay {
      GeometryReader { geometry in
        if !wide, transportLayout.frames[.surface] != nil {
          let origin = geometry.frame(in: .global).origin
          TransportPresentation(expanded: expanded, lift: transportLift, presenting: $transportAnimating,
            transport: transportLayout.frames.mapValues { $0.offsetBy(dx: -origin.x, dy: -origin.y) },
            size: geometry.size, origin: origin)
        }
      }
    }
  }
}
private struct LibraryBrowser: View {
  @Environment(AppModel.self) private var model
  var body: some View {
    let path = model.path
    NavigationStack(
      path: Binding(get: { model.path }, set: { model.setNativePath($0, from: path) })
    ) {
      LibraryHome().modifier(LibraryNavigationControls())
        .navigationDestination(for: String.self) { owner in
          LibraryDestination(owner: owner).modifier(LibraryNavigationControls())
        }
    }
  }
}

private let tabletColumnHeaderHeight: CGFloat = 44

private struct TabletColumnDivider: View {
  @Environment(\.textampTheme) private var theme
  var body: some View {
    Rectangle().fill(theme.columnDivider)
      .frame(width: 1).accessibilityHidden(true).allowsHitTesting(false)
  }
}

private struct TabletLibrary: View {
  @Environment(AppModel.self) private var model
  let rootWidth: CGFloat
  @Binding var sidebarVisible: Bool
  @State private var visibleColumn: String?

  var body: some View {
    NavigationSplitView(
      columnVisibility: Binding(
        get: { sidebarVisible ? .all : .detailOnly },
        set: { sidebarVisible = $0 != .detailOnly }
      )
    ) {
      LibraryHome(sidebar: true)
        .overlay(alignment: .trailing) {
          TabletColumnDivider().ignoresSafeArea(.container, edges: .top)
        }
        .navigationSplitViewColumnWidth(min: 208, ideal: rootWidth, max: 300)
    } detail: {
      GeometryReader { geometry in
        let path = model.path
        if path.isEmpty {
          ContentUnavailableView("Choose a Library Category", systemImage: "square.stack")
        } else {
          let slots = min(path.count, max(1, Int(geometry.size.width / 300)))
          let width = geometry.size.width / CGFloat(slots)
          ScrollViewReader { reader in
            ScrollView(.horizontal) {
              HStack(spacing: 0) {
                ForEach(path, id: \.self) { owner in
                  LibraryDestination(owner: owner, showsSelection: owner != path.last, inWorkspace: true)
                    .frame(width: width, height: geometry.size.height)
                    .overlay(alignment: .trailing) {
                      if owner != path.last { TabletColumnDivider() }
                    }
                    .id(owner)
                }
              }.scrollTargetLayout()
            }
            .defaultScrollAnchor(.trailing)
            .scrollPosition(id: $visibleColumn, anchor: .trailing)
            .onChange(of: path, initial: true) { _, newPath in visibleColumn = newPath.last }
            .task(id: geometry.size.width) {
              await Task.yield()
              guard !Task.isCancelled, let last = path.last else { return }
              reader.scrollTo(last, anchor: .trailing)
            }
          }
        }
      }
      .background(model.theme.baseFill)
      .navigationBarTitleDisplayMode(.inline)
      .toolbar(sidebarVisible ? .hidden : .visible, for: .navigationBar)
    }
    .navigationSplitViewStyle(.balanced)
  }

}

private struct LibraryControlIcon: View {
  let symbol: String
  var title: String?
  var compact = false
  var body: some View {
    Group {
      if let title { Label(title, systemImage: symbol).labelStyle(.iconOnly) }
      else { Image(systemName: symbol) }
    }.font(compact ? .title3 : .title2)
      .frame(width: compact ? 44 : 48, height: compact ? 44 : 48)
      .contentShape(Rectangle())
  }
}

private struct LibraryHeaderRow<Content: View>: View {
  @ViewBuilder var content: Content

  var body: some View {
    HStack(spacing: 0) { content }
      .frame(minHeight: tabletColumnHeaderHeight)
      .padding(.leading, 12).padding(.trailing, 4)
  }
}

private struct CloseLibraryColumn: View {
  @Environment(AppModel.self) private var model
  let owner: String
  var body: some View {
    Button {
      if let index = model.path.firstIndex(of: owner) {
        model.setPath(Array(model.path.prefix(index)))
      }
    } label: {
      Image(systemName: "xmark").frame(width: 44, height: tabletColumnHeaderHeight)
        .contentShape(Rectangle())
    }
    .buttonStyle(.plain).accessibilityLabel("Close Column")
    .contextMenu {
      Button("Return to Previous Location", systemImage: "arrow.uturn.backward") {
        model.send("return_to_previous_location")
      }.disabled(model.snapshot?.canReturn != true)
    }
  }
}

private struct LibraryDestination: View {
  @Environment(AppModel.self) private var model
  let owner: String
  var showsSelection = false
  var inWorkspace = false

  var body: some View {
    if let column = model.snapshot?.columns.first(where: { $0.id == owner }) {
      if column.radio == true {
        titledColumn(column.compactTitle ?? column.title, namedDestination: column.namedDestination == true) {
          RadioScreen(column: column, showsSelection: showsSelection)
        }
      } else if column.detail {
        titledColumn("Details") { DetailScreen() }
      } else {
        ColumnScreen(owner: owner, showsSelection: showsSelection, inWorkspace: inWorkspace)
      }
    } else if owner.hasPrefix("track:") {
      titledColumn("Details") { DetailScreen() }
    }
  }

  private func titledColumn(_ title: String, namedDestination: Bool = false, @ViewBuilder content: () -> some View) -> some View {
    VStack(spacing: 0) {
      LibraryHeaderRow {
        ColumnHeading(title: title, namedDestination: namedDestination)
        if inWorkspace { CloseLibraryColumn(owner: owner) }
      }
      .accessibilityElement(children: .contain)
      .accessibilityIdentifier("workspace-column-header-\(title)")
      if inWorkspace { Divider() }
      content()
    }
    .background(model.theme.baseFill)
    .navigationTitle("").navigationBarTitleDisplayMode(.inline)
  }
}

struct MainBottomBar: View {
  @Environment(AppModel.self) var model
  @Environment(\.queueColumn) private var queueColumn
  var body: some View {
    HStack(spacing: 0) {
      if queueColumn == nil { destination("Library", symbol: "square.stack", tab: "library") }
      Button {
        model.search()
      } label: {
        Image(systemName: "magnifyingglass").frame(maxWidth: .infinity, minHeight: 44)
          .contentShape(Rectangle())
      }.accessibilityLabel("Search").accessibilityIdentifier("bottom-search")
      Button {
        model.open("palette")
      } label: {
        Image(systemName: "command").frame(maxWidth: .infinity, minHeight: 44)
          .contentShape(Rectangle())
      }.accessibilityLabel("Command Palette")
      destination("Play Queue", symbol: "music.note.list", tab: "queue")
    }
    .font(.system(size: 22)).buttonStyle(.plain).foregroundStyle(model.theme.text)
    .padding(.horizontal, 12).padding(.vertical, 5)
    .background(model.theme.system ? AnyShapeStyle(.bar) : model.theme.baseFill).overlay(
      alignment: .top
    ) { Divider() }
    .accessibilityElement(children: .contain)
    .accessibilityIdentifier("main-controls")

  }
  private func destination(_ label: String, symbol: String, tab: String) -> some View {
    let dock = tab == "queue" ? queueColumn : nil
    let selected = dock?.wrappedValue ?? (model.libraryTab == tab)
    return Button {
      if let dock {
        if model.settings || model.sheet != nil {
          model.closeSheet()
          dock.wrappedValue = true
        } else {
          dock.wrappedValue.toggle()
        }
      } else {
        let reselectingLibrary = tab == "library" && model.tab == "library"
          && model.sheet == nil && !model.settings
        model.selectTab(tab)
        if reselectingLibrary {
          model.setPath([])
        }
      }
    } label: {
      Image(systemName: symbol).frame(width: 56, height: 44)
        .foregroundStyle(selected ? model.theme.tint : model.theme.muted)
        .background(
          selected ? model.theme.tint.opacity(0.14) : .clear,
          in: RoundedRectangle(cornerRadius: 14)
        ).frame(maxWidth: .infinity).contentShape(Rectangle())
    }.accessibilityLabel(label).accessibilityIdentifier("bottom-\(tab)")
      .accessibilityAddTraits(selected ? .isSelected : [])
  }
}
private func categorySymbol(_ title: String, section: String? = nil) -> String {
  if section == "Playlists" { return "music.note.list" }
  switch title {
  case "Artists", "Favorite artists": return "music.mic"
  case "Albums", "All": return "square.stack"
  case "Playlists": return "music.note.list"
  case "Tracks", "Favorite tracks": return "music.note"
  case "Downloads": return "arrow.down.circle"
  case "Folders": return "folder"
  case "Genres", "Artist genres": return "guitars"
  case "Decades", "Years": return "calendar"
  case "Radio": return "dot.radiowaves.left.and.right"
  case "AI": return "sparkles"
  case "Random": return "shuffle"
  case "Favourites": return "heart"
  case "Top Rated": return "star"
  case "Recently Added": return "plus.circle"
  case "Recently Played": return "clock"
  case "Most Played": return "chart.bar"
  case "Compilations": return "square.stack.3d.up"
  case "Singles": return "opticaldisc"
  case "Journey": return "point.topleft.down.to.point.bottomright.curvepath"
  case "Describe Music", "Search lyrics…", "Search lyrics...": return "magnifyingglass"
  case "Tempo": return "metronome"
  case "Energy": return "bolt"
  case "Musical key": return "pianokeys"
  default: return "tag"
  }
}
struct LibraryHome: View {
  @Environment(AppModel.self) var model
  var sidebar = false
  @AppStorage("libraryRadioStation") private var preferredStation = "randomAlbum"
  @State private var expandedGroups = Set<String>()
  private var stations: [CommandButton] { model.snapshot?.libraryStations ?? [] }
  private var station: CommandButton? {
    stations.first { $0.value == preferredStation } ?? stations.first { $0.value == "randomAlbum" } ?? stations.first
  }
  var body: some View {
    VStack(spacing: 0) {
      LibraryHeaderRow {
        Group {
          if sidebar {
            Text("Library").font(.headline).lineLimit(1)
              .frame(maxWidth: .infinity, alignment: .leading)
              .accessibilityAddTraits(.isHeader)
          } else {
            ColumnHeading(title: "Library", namedDestination: false)
          }
        }.accessibilityIdentifier("library-heading")
        if let station {
          Menu {
            Picker("Radio Station", selection: Binding(get: { station.value ?? "" }, set: { preferredStation = $0 })) {
              ForEach(stations) { option in
                Text(option.label).tag(option.value ?? "")
              }
            }
          } label: {
            LibraryControlIcon(symbol: "dot.radiowaves.left.and.right", compact: sidebar)
          } primaryAction: {
            model.invoke(station.token)
          }
          .menuIndicator(.hidden)
          .accessibilityLabel("Radio")
          .accessibilityValue(station.label)
          .accessibilityHint("Touch and hold to choose a station")
          .accessibilityIdentifier("library-radio")
        }
        Button { model.open("library_picker") } label: {
          LibraryControlIcon(symbol: "books.vertical", compact: sidebar)
        }.accessibilityLabel("Switch Library")
        Button { model.open("settings") } label: {
          LibraryControlIcon(symbol: "gearshape", compact: sidebar)
        }.accessibilityLabel("Settings")
      }
      .font(.title2)
      if sidebar { Divider() }
      ScrollViewReader { reader in
        Group {
          if sidebar {
            destinations.listStyle(.sidebar).contentMargins(.horizontal, 6, for: .scrollContent)
          } else {
            destinations.listStyle(.plain)
          }
        }
        .onChange(of: expandedGroups) { previous, current in
          if let title = current.subtracting(previous).first,
            let first = groups.first(where: { $0.title == title })?.destinations.first
          {
            reader.scrollTo(first.id)
          }
        }
      }
    }
    .font(.body).foregroundStyle(model.theme.text)
    .environment(\.defaultMinListRowHeight, 44)
    .scrollIndicators(.visible).scrollContentBackground(.hidden).background(model.theme.isBrushedAluminum ? model.theme.baseFill : AnyShapeStyle(libraryBackground))
    .navigationTitle("").navigationBarTitleDisplayMode(.inline)
  }
  private var destinations: some View {
    List {
      if model.snapshot == nil || model.snapshot?.libraryLoading == true {
        ProgressView("Opening Library…")
          .frame(maxWidth: .infinity, minHeight: 120)
          .listRowBackground(Color.clear)
          .accessibilityIdentifier("library-loading")
      } else if model.snapshot?.libraries.isEmpty == true {
        ContentUnavailableView {
          Label("Your Music Library", systemImage: "music.note.house")
            .foregroundStyle(model.theme.text)
        } description: {
          Text("Connect to Navidrome or a WebDAV library.")
            .foregroundStyle(model.theme.muted)
        } actions: {
          Button("Add Library") { model.open("settings") }
            .buttonStyle(.borderedProminent)
            .tint(model.theme.tint)
            .foregroundStyle(model.theme.onTint)
        }
        .listRowBackground(model.theme.isBrushedAluminum ? Color.clear : libraryBackground)
      }
      ForEach(model.snapshot?.libraryLoading == false ? groups : []) { group in
        if let title = group.title,
          let command = model.snapshot?.sidebarGroups.first(where: { $0.label == title }) {
          let active =
            sidebar && !model.path.isEmpty
            && (model.snapshot?.columns.first.map { ($0.sidebar == true || $0.radio == true) && $0.title == title }
              == true
              || group.destinations.contains { $0.active == true })
          HStack(spacing: 0) {
            Button {
              model.invoke(command.token)
            } label: {
              categoryLabel(title, selected: active).frame(maxWidth: .infinity, alignment: .leading)
                .frame(minHeight: 44).contentShape(Rectangle())
            }.buttonStyle(.plain).accessibilityAddTraits(active ? .isSelected : [])
              .accessibilityIdentifier("sidebar-open-\(title)")
            if command.canExpand == true {
              Button {
                withAnimation(
                  UIAccessibility.isReduceMotionEnabled ? nil : .easeInOut(duration: 0.2)
                ) {
                  if expandedGroups.contains(title) {
                    expandedGroups.remove(title)
                  } else {
                    expandedGroups.insert(title)
                  }
                }
              } label: {
                Image(systemName: expandedGroups.contains(title) ? "chevron.down" : "chevron.right")
                  .font(.footnote.weight(.semibold)).foregroundStyle(
                    active ? model.theme.onTint : model.theme.muted
                  )
                  .frame(width: 44, height: 44)
                  .contentShape(Rectangle())
              }.buttonStyle(.plain)
                .accessibilityLabel(
                  "\(expandedGroups.contains(title) ? "Collapse" : "Expand") \(title)"
                )
                .accessibilityIdentifier("sidebar-disclosure-\(title)")
            }
          }
          .listRowBackground(rowBackground(selected: active))
          .listRowInsets(EdgeInsets(top: 4, leading: 16, bottom: 4, trailing: 16))
          if expandedGroups.contains(title),
            command.canExpand == true
          {
            ForEach(group.destinations) { destination($0).padding(.leading, 20).id($0.id) }
          }
        } else {
          ForEach(group.destinations) { destination($0) }
        }
      }
    }
  }
  private func destination(_ item: CommandButton) -> some View {
    let active = sidebar && !model.path.isEmpty && item.active == true
    return Button {
      model.invoke(item.token)
    } label: {
      categoryLabel(item.label, section: item.section, selected: active)
        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        .contentShape(Rectangle())
    }.buttonStyle(.plain).listRowBackground(rowBackground(selected: active))
      .accessibilityAddTraits(active ? .isSelected : [])
      .contextMenu {
        ForEach(item.context ?? []) { command in
          Button(command.label, systemImage: "arrow.down.circle") { model.invoke(command.token) }
            .disabled(command.disabled != nil)
        }
      }
      .listRowInsets(EdgeInsets(top: 4, leading: 16, bottom: 4, trailing: 16))
  }
  @ViewBuilder private func rowBackground(selected: Bool) -> some View {
    if sidebar {
      RoundedRectangle(cornerRadius: 10)
        .fill(selected ? model.theme.tint : Color.clear)
    } else {
      model.theme.card
    }
  }
  private var libraryBackground: Color {
    model.theme.system ? Color(uiColor: .systemBackground) : model.theme.bg
  }
  private func categoryLabel(_ title: String, section: String? = nil, selected: Bool) -> some View {
    HStack(spacing: 12) {
      Image(systemName: categorySymbol(title, section: section)).font(.system(size: 20))
        .frame(width: 26).accessibilityHidden(true)
      Text(title).fontWeight(selected ? .semibold : .regular)
    }.foregroundStyle(selected ? model.theme.onTint : model.theme.text)
  }
  private var groups: [RootGroup] {
    var groups: [RootGroup] = []
    var indices: [String: Int] = [:]
    for item in model.snapshot?.roots ?? [] {
      guard let title = item.section, title != "Library" else {
        groups.append(RootGroup(id: item.id, title: nil, destinations: [item]))
        continue
      }
      let index: Int
      if let existing = indices[title] {
        index = existing
      } else {
        index = groups.count
        indices[title] = index
        groups.append(RootGroup(id: "group:\(title)", title: title, destinations: []))
      }
      groups[index].destinations.append(item)
    }
    return groups
  }
  private struct RootGroup: Identifiable {
    let id: String
    let title: String?
    var destinations: [CommandButton]
  }
}
struct ActivityStatus: View {
  @Environment(AppModel.self) var model
  @State private var expanded = false
  private var notice: StatusNotice? { model.snapshot?.notice }
  private var message: String? { notice?.message ?? model.snapshot?.loading }
  var body: some View {
    if model.sheet != "palette" {
      VStack(spacing: 0) {
        if model.snapshot?.offline == true {
          Label("Offline", systemImage: "wifi.slash")
            .font(.footnote.weight(.semibold))
            .foregroundStyle(model.theme.text)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .background(model.theme.card)
            .accessibilityIdentifier("offline-status")
        }
        if let message {
          HStack(spacing: 10) {
            if notice == nil {
              ProgressView().controlSize(.small).accessibilityHidden(true)
            } else {
              Image(
                systemName: notice?.kind == "error" ? "exclamationmark.circle" : "checkmark.circle"
              )
              .foregroundStyle(notice?.kind == "error" ? model.theme.error : model.theme.muted)
              .accessibilityHidden(true)
            }
            Button {
              if let action = notice?.action
                ?? (notice == nil ? model.snapshot?.activityAction : nil)
              {
                model.invoke(action)
              } else {
                expanded.toggle()
              }
            } label: {
              Text(message).font(.footnote).lineLimit(expanded ? nil : 2)
                .frame(maxWidth: .infinity, alignment: .leading)
            }.buttonStyle(.plain)
            if let action = notice?.action, let label = notice?.actionLabel {
              Button(label) { model.invoke(action) }
                .buttonStyle(.bordered)
                .foregroundStyle(model.theme.text)
                .accessibilityIdentifier("retry-library")
            }
            if notice != nil {
              Button {
                model.send("dismiss_notice", ["message": message])
              } label: {
                Image(systemName: "xmark").font(.caption.weight(.semibold))
                  .frame(width: 44, height: 44)
              }.buttonStyle(.plain).accessibilityLabel("Dismiss Status")
            }
          }.foregroundStyle(model.theme.text).padding(.leading, 14).padding(.trailing, 4)
            .padding(.vertical, notice == nil ? 10 : 0)
            .background(
              model.theme.system
                ? AnyShapeStyle(.regularMaterial) : AnyShapeStyle(model.theme.card),
              in: RoundedRectangle(cornerRadius: 14)
            )
            .padding(.horizontal, 12).padding(.vertical, 4)
            .task(id: notice) {
              guard let notice, notice.actionLabel == nil else { return }
              let seconds =
                UIAccessibility.isVoiceOverRunning ? 12 : (notice.kind == "error" ? 8 : 4)
              do { try await Task.sleep(for: .seconds(seconds)) } catch { return }
              model.send("dismiss_notice", ["message": notice.message])
            }
            .onChange(of: message) { _, _ in expanded = false }
            .onChange(of: notice) { _, value in
              if let value {
                UIAccessibility.post(notification: .announcement, argument: value.message)
              }
            }
        }
      }
    }
  }
}
struct ColumnScreen: View {
  @Environment(AppModel.self) var model
  let owner: String
  var showsSelection = false
  var inWorkspace = false
  @State private var filtering = false
  @State private var query = ""
  @State private var selectionOwner: String?
  @State private var readingAlbumNotes = false
  @State private var descriptionQuery = ""
  @FocusState private var filterFocus: Bool
  var column: Column? { model.snapshot?.columns.first { $0.id == owner } }
  var body: some View {
    Group {
      if let column {
        VStack(spacing: 0) {
          if inWorkspace || filtering || column.albumArtworkKey == nil {
            LibraryHeaderRow {
              if filtering || column.albumArtworkKey == nil { columnTitle(column) }
              if inWorkspace {
                if !filtering { columnActions(column) }
                CloseLibraryColumn(owner: owner)
              }
            }
          }
          if inWorkspace { Divider() }
          if column.descriptionPrompt == true {
            HStack {
              TextField("Describe Music", text: $descriptionQuery).submitLabel(.search).onSubmit {
                model.send("describe", ["query": descriptionQuery])
              }
              Button {
                model.send("describe", ["query": descriptionQuery])
              } label: {
                Image(systemName: "arrow.up.circle.fill")
              }.disabled(descriptionQuery.isEmpty).accessibilityLabel("Search description")
            }.padding().background(model.theme.card)
          }
          GeometryReader { pane in
            ColumnList(
              column: column, paneWidth: pane.size.width, showsSelection: showsSelection,
              keyboardTarget: model.path.last == owner && !filterFocus && model.sheet == nil,
              selecting: selectionOwner == owner,
              beginSelecting: { selectionOwner = owner },
              showAlbumNotes: { readingAlbumNotes = true },
              dismissFilterKeyboard: { filterFocus = false })
          }
          if let summary = column.summary {
            Text(summary).font(.footnote).foregroundStyle(model.theme.muted)
              .accessibilityIdentifier("download-summary")
              .frame(maxWidth: .infinity).padding(.vertical, 8).background(model.theme.card)
          }
          if column.count == 0 && model.snapshot?.loading == nil {
            Text(query.isEmpty ? "No items" : "No matches").foregroundStyle(model.theme.muted)
              .padding()
          }
        }.background(model.theme.baseFill)
          .navigationTitle("").navigationBarTitleDisplayMode(.inline)
          .toolbar {
            if !inWorkspace && !filtering {
              ToolbarItemGroup(placement: .topBarTrailing) { columnActions(column) }
            }
          }
          .sheet(isPresented: $readingAlbumNotes) {
            if let detail = column.albumPage { AlbumNotesReader(detail: detail) }
          }
          .onChange(of: column.filterActive, initial: true) { _, active in
            if active == true && !filtering && column.albumArtworkKey == nil {
              query = column.filter
              filtering = true
            }
          }
      } else {
        ProgressView()
      }
    }
  }
  @ViewBuilder private func columnActions(_ column: Column) -> some View {
    if let detail = column.albumPage {
      ForEach((column.playback ?? []).dropFirst()) { action in
        Button { model.invoke(action.token) } label: {
          LibraryControlIcon(symbol: action.symbol ?? "plus", title: action.label)
        }.accessibilityLabel(action.label)
      }
      if let download = detail.actions.first(where: { $0.symbol == "arrow.down.circle" }) {
        Button { model.invoke(download.token) } label: {
          LibraryControlIcon(symbol: "arrow.down.circle", title: download.label)
        }.accessibilityLabel(download.label)
      }
    }
    if let mix = column.artistHeader?.mix {
      Button { model.invoke(mix.token) } label: {
        LibraryControlIcon(symbol: "dot.radiowaves.left.and.right")
      }.accessibilityLabel(mix.label)
    }
    playlistControls(column)
    viewOptions(column)
  }
  @ViewBuilder private func playlistControls(_ column: Column) -> some View {
    if selectionOwner == column.id {
      Button {
        model.send("clear_selection", ["owner": column.id])
        selectionOwner = nil
      } label: {
        LibraryControlIcon(symbol: "checkmark")
      }.accessibilityLabel("Done")
        .accessibilityIdentifier("selection-done")
    } else if column.playlistEditing != true && column.albumArtworkKey == nil {
      ActionButtons(actions: column.playback ?? [], compact: true)
    }
    if let controls = column.playlistEditControls, let toggle = controls.first {
      Button {
        model.invoke(toggle.token)
      } label: {
        LibraryControlIcon(symbol: column.playlistEditing == true ? "checkmark" : "pencil")
      }.accessibilityLabel(toggle.label)
        .accessibilityIdentifier("playlist-edit")
      if column.playlistEditing == true {
        let removal = controls.first { $0.symbol == "trash" }
        Button(role: .destructive) {
          if let removal { model.invoke(removal.token) }
        } label: {
          LibraryControlIcon(symbol: "trash")
        }
        .disabled(removal == nil)
        .accessibilityLabel("Remove from Playlist")
        .accessibilityIdentifier("playlist-remove")
        let moves = controls.dropFirst().filter { $0.symbol != "trash" }
        if !moves.isEmpty {
          Menu {
            ForEach(moves) { action in
              Button(action.label) { model.invoke(action.token) }
            }
          } label: {
            LibraryControlIcon(symbol: "ellipsis.circle")
          }.accessibilityLabel("Selected Tracks")
        }
      }
    }
  }
  @ViewBuilder private func columnTitle(_ column: Column) -> some View {
    if filtering {
      HStack {
        TextField("Filter \(column.title)", text: Binding(
          get: { query },
          set: { value in
            guard value != query else { return }
            query = value
            model.send("filter", ["owner": owner, "query": value])
          }
        )).textInputAutocapitalization(
          .never
        )
        .autocorrectionDisabled().focused($filterFocus)
        .submitLabel(.done).onSubmit { filterFocus = false }
        .onAppear { filterFocus = true }
        Button {
          query = ""
          filtering = false
          filterFocus = false
          model.send("filter", ["owner": owner, "query": ""])
        } label: {
          LibraryControlIcon(symbol: "xmark.circle.fill")
        }.accessibilityLabel("Close Filter")
      }.frame(minHeight: 44)
    } else {
      Button {
        query = column.filter
        filtering = true
      } label: {
        ColumnHeading(title: column.compactTitle ?? column.title, namedDestination: column.namedDestination == true || column.artistHeader != nil, artistHeading: column.artistHeader != nil)
          .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
          .contentShape(Rectangle())
      }.buttonStyle(.plain).accessibilityLabel(column.title).accessibilityIdentifier("column-title")
        .accessibilityHint("Filter this list")
    }
  }
  @ViewBuilder private func viewOptions(_ column: Column) -> some View {
    if column.sidebar != true && !column.options.isEmpty {
      Menu {
        ForEach(column.options.menuSections.indices, id: \.self) { index in
          Section {
            ForEach(column.options.menuSections[index]) { item in
              Button {
                model.invoke(item.token)
              } label: {
                Label(item.label, systemImage: item.checked == true ? "checkmark" : "circle")
              }
            }
          }
        }
      } label: {
        LibraryControlIcon(symbol: "slider.horizontal.3", title: "View Options")
      }.accessibilityLabel("View Options")
    }
  }

}
/// Only an overflowing title owns an animation task. SwiftUI cancels it on navigation or resize.
struct ColumnHeading: View {
  let title: String
  let namedDestination: Bool
  var artistHeading = false
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @State private var textWidth: CGFloat = 0
  @State private var offset: CGFloat? = nil

  var body: some View {
    let heading = title.prefix(1).uppercased() + title.dropFirst()
    Group {
      if namedDestination {
        ViewThatFits(in: .horizontal) {
          Text(heading).font(artistHeading ? .largeTitle.bold() : .headline)
            .lineLimit(1).fixedSize(horizontal: true, vertical: false)
          scrollingTitle(heading).font(.subheadline.weight(artistHeading ? .bold : .semibold))
        }
      } else {
        scrollingTitle(heading).font(.largeTitle.bold())
      }
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .accessibilityRepresentation { Text(heading).accessibilityAddTraits(.isHeader) }
    .id(title)
  }

  private func scrollingTitle(_ heading: String) -> some View {
    Text(heading).lineLimit(1).truncationMode(.middle)
      .frame(maxWidth: .infinity, alignment: .leading)
      .opacity(offset == nil ? 1 : 0)
      .overlay {
        GeometryReader { geometry in
          let distance = max(0, textWidth - geometry.size.width)
          Text(heading).lineLimit(1).fixedSize()
            .onGeometryChange(for: CGFloat.self) {
              $0.size.width
            } action: {
              textWidth = $0
            }
            .offset(x: -(offset ?? 0)).opacity(offset == nil ? 0 : 1)
            .accessibilityHidden(true)
            .task(id: reduceMotion ? 0 : distance) {
              offset = nil
              guard !reduceMotion, distance > 0 else { return }
              do {
                while !Task.isCancelled {
                  try await Task.sleep(for: .seconds(10))
                  offset = 0
                  try await Task.sleep(for: .milliseconds(100))
                  let duration = Double(distance) / 30
                  withAnimation(.linear(duration: duration)) { offset = distance }
                  try await Task.sleep(for: .seconds(duration + 2))
                  offset = nil
                }
              } catch { /* The replacement task starts at the centered truncation. */  }
            }
        }.clipped().allowsHitTesting(false).accessibilityHidden(true)
      }
  }
}

struct RadioScreen: View {
  @Environment(AppModel.self) var model
  let column: Column
  var showsSelection = false
  var body: some View {
    Group {
      if let detail = column.stationDetails, let input = detail.input {
        if input == "artists" {
          UtilitySheet()
        } else {
          VStack(spacing: 0) {
            Text(detail.text).font(.subheadline).foregroundStyle(model.theme.muted)
              .frame(maxWidth: .infinity, alignment: .leading).padding()
            JourneyFormView()
          }
        }
      } else {
        stationList
      }
    }.background(model.theme.baseFill)

  }
  private var stationList: some View {
    List {
      if let detail = column.stationDetails {
        ActionButtons(actions: detail.actions).listRowBackground(model.theme.card)
        if !detail.text.isEmpty { Text(detail.text).listRowBackground(model.theme.card) }
      } else {
        if let description = column.stationDescription {
          Text(description).font(.subheadline).foregroundStyle(model.theme.muted)
            .listRowBackground(model.theme.card)
        }
        ForEach(column.rows) { row in
          HStack(spacing: 0) {
            Button {
              if let token = row.activation { model.invoke(token) }
            } label: {
              HStack(spacing: 14) {
                Image(systemName: "dot.radiowaves.left.and.right")
                Text(row.title).frame(maxWidth: .infinity, alignment: .leading)
              }.padding(.vertical, 10).contentShape(Rectangle())
            }.buttonStyle(.plain).accessibilityIdentifier("station-play-\(row.key)")
            if let token = row.disclosure {
              Button {
                model.invoke(token)
              } label: {
                Image(systemName: "chevron.right").font(.body).frame(width: 44, height: 44)
              }.buttonStyle(.plain).foregroundStyle(model.theme.muted)
                .accessibilityLabel("About \(row.title)")
            }
          }.listRowBackground(
            showsSelection && row.selected ? model.theme.tint.opacity(0.18) : model.theme.card)
        }

      }
    }.scrollContentBackground(.hidden).background(model.theme.baseFill)
  }
}
struct MusicRowView: View {
  @Environment(AppModel.self) var model
  let row: MusicRow
  var selected = false
  private var text: Color { selected ? model.theme.onTint : model.theme.text }
  private var secondary: Color { selected ? model.theme.onTint : model.theme.muted }
  var body: some View {
    HStack(spacing: 12) {
      if let art = row.artwork {
        Artwork(id: art).frame(width: 52, height: 52).clipShape(RoundedRectangle(cornerRadius: 6))
          .opacity(row.pending == nil ? 1 : 0.45)
      } else {
        Image(
          systemName: row.kind == "category"
            ? categorySymbol(row.title)
            : row.kind == "artist"
              ? "person" : row.kind == "track" ? "music.note" : "folder"
        ).frame(width: 24).foregroundStyle(secondary).accessibilityHidden(true)
      }
      VStack(alignment: .leading, spacing: 4) {
        Text(title).fontWeight(selected ? .semibold : .regular).foregroundStyle(
          row.pending == nil ? text : secondary
        )
        .lineLimit(2)
        if let pending = row.pending {
          Text(pending).font(.subheadline).foregroundStyle(secondary).lineLimit(2)
        } else if !row.subtitle.isEmpty {
          Text(row.subtitle).font(.subheadline).foregroundStyle(secondary).lineLimit(1)
        }
      }
      Spacer(minLength: 0)
      if let cancel = row.cancel {
        Button {
          model.invoke(cancel.token)
        } label: {
          Image(systemName: "xmark").font(.body).frame(width: 44, height: 44)
        }.buttonStyle(.borderless).accessibilityLabel("Cancel Download \(row.title)")
          .accessibilityIdentifier("download-cancel-\(row.key)")
      } else {
        Image(systemName: "chevron.right").font(.caption).foregroundStyle(secondary)
          .accessibilityHidden(true)
      }
    }.padding(.vertical, 4).contentShape(Rectangle())
  }
  var title: String { row.albumCaption }
}
private struct AlbumArtistBiographyButton: View {
  @Environment(AppModel.self) private var model
  let columnID: String
  let side: CGFloat

  var body: some View {
    let command = model.snapshot?.columns.first { $0.id == columnID }?.albumPage?.artistBio
    Group {
      if let command, command.mark != nil {
        ArtistWebSearchButton(command: command).buttonStyle(.plain)
      } else if let command {
        Button { model.invoke(command.token) } label: {
          Image(systemName: "person").font(.title).frame(width: side, height: side)
            .contentShape(Rectangle())
        }.buttonStyle(.plain).accessibilityLabel(command.label)
      } else { Color.clear }
    }.frame(width: side, height: side)
  }
}

struct AlbumPageHeader: View {
  @Environment(AppModel.self) private var model
  @ScaledMetric(relativeTo: .body) private var infoButtonSide: CGFloat = 52
  let column: Column
  let detail: Details?
  let showAlbumNotes: () -> Void

  var body: some View {
    VStack(spacing: 8) {
      Artwork(id: column.albumDetailsArtwork)
        .aspectRatio(1, contentMode: .fit).frame(maxWidth: 340)
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .accessibilityLabel("Album artwork")
        .task(id: column.albumArtworkKey) {
          if let key = column.albumArtworkKey { model.send("artwork", ["owner": column.id, "keys": [key]]) }
        }
      if let detail {
        Text(detail.title).font(.title2.bold()).multilineTextAlignment(.center).padding(.top, 8)
        if let artist = detail.albumArtist {
          Button(artist.label) { model.invoke(artist.token) }.font(.title3)
        } else if !detail.subtitle.isEmpty { Text(detail.subtitle).font(.title3) }
        let metadata = ["Genres", "Year"].compactMap { label in
          detail.rows.first { $0.first == label }?.dropFirst().first
        }.joined(separator: " · ")
        if !metadata.isEmpty { Text(metadata).font(.subheadline).foregroundStyle(model.theme.muted) }
        HStack(spacing: 12) {
          AlbumArtistBiographyButton(columnID: column.id, side: infoButtonSide)
          if let play = column.playback?.first {
            Button { model.invoke(play.token) } label: {
              Label("Play", systemImage: "play.fill").fontWeight(.semibold).frame(minWidth: 86)
            }.buttonStyle(.borderedProminent).buttonBorderShape(.capsule)
              .foregroundStyle(model.theme.onTint).accessibilityLabel(play.label)
              .contextMenu {
                ForEach(column.playback ?? []) { action in
                  Button(action.label) { model.invoke(action.token) }
                  ForEach(action.context ?? []) { alternate in
                    Button(alternate.label) { model.invoke(alternate.token) }
                  }
                }
              }
          }
          Group {
            if detail.review?.isEmpty == false || detail.albumURL != nil {
              Button(action: showAlbumNotes) {
                Image(systemName: "opticaldisc").font(.title).frame(width: infoButtonSide, height: infoButtonSide).contentShape(Rectangle())
              }.buttonStyle(.plain).accessibilityLabel("Album Info").accessibilityIdentifier("album-notes")
            } else { Color.clear }
          }.frame(width: infoButtonSide, height: infoButtonSide)
        }.controlSize(.large).padding(.vertical, 4)
      }
    }.frame(maxWidth: .infinity).padding(.vertical, 16).padding(.horizontal, 3)
  }

}

private struct AlbumNotesReader: View {
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  @State private var height: PresentationDetent = .large
  let detail: Details

  var body: some View {
    NavigationStack {
      ScrollView {
        VStack(alignment: .leading, spacing: 16) {
          Text(detail.title).font(.title.bold())
          Text((detail.review ?? []).reduce(into: AttributedString()) { result, passage in
            var part = AttributedString(passage.text)
            if passage.italic { part.inlinePresentationIntent = .emphasized }
            result.append(part)
          }).textSelection(.enabled).accessibilityIdentifier("album-review")
          if let url = detail.albumURL, let host = url.host {
            Link(host, destination: url).accessibilityIdentifier("album-source-link")
          }
        }.frame(maxWidth: .infinity, alignment: .leading).padding()
      }.background(model.theme.baseFill).foregroundStyle(model.theme.text)
        .navigationTitle("Album Info").navigationBarTitleDisplayMode(.inline)
        .toolbar { SheetDismissToolbarItem { dismiss() } }
    }.tint(model.theme.tint).modifier(PlaybackSheetDetents(height: $height, dismiss: { dismiss() }))
  }
}

struct ColumnRecommendations: View {
  let columns: Int
  @Environment(AppModel.self) private var model
  let albums: [CommandButton]
  let artists: [CommandButton]
  let message: String

  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      if !albums.isEmpty || !message.isEmpty {
        tiles("Similar Albums", items: albums, portraits: false)
        if !message.isEmpty { Text(message).font(.footnote).foregroundStyle(model.theme.muted) }
      }
      if !artists.isEmpty { tiles("Similar Artists", items: artists, portraits: true) }
    }.frame(maxWidth: .infinity, alignment: .leading).foregroundStyle(model.theme.text)
  }

  private func tiles(_ title: String, items: [CommandButton], portraits: Bool) -> some View {
    VStack(alignment: .leading, spacing: 8) {
      Text(title).font(.headline).accessibilityAddTraits(.isHeader)
      LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 10), count: columns), alignment: .leading, spacing: 12) {
        ForEach(items) { item in
          Button { model.invoke(item.token) } label: {
            VStack(alignment: portraits ? .center : .leading, spacing: 5) {
              Color.clear.aspectRatio(1, contentMode: .fit)
                .overlay { Artwork(id: item.artwork) }
                .clipShape(portraits ? AnyShape(Circle()) : AnyShape(RoundedRectangle(cornerRadius: 6)))
              Text(item.subtitle ?? item.label).font(.subheadline).lineLimit(2)
                .multilineTextAlignment(portraits ? .center : .leading)
              if let artist = item.description { Text(artist).font(.caption).foregroundStyle(model.theme.muted).lineLimit(1) }
            }.frame(maxWidth: .infinity, alignment: portraits ? .center : .leading).contentShape(Rectangle())
          }.buttonStyle(.plain).accessibilityLabel(item.label)
        }
      }
    }
  }
}

struct RelatedMusicSection: View {
  @Environment(AppModel.self) private var model
  let detail: Details
  var body: some View {
    if !detail.related.isEmpty || !detail.similar.isEmpty || !detail.message.isEmpty {
      VStack(alignment: .leading, spacing: 10) {
        Text(detail.relatedTitle).font(.title3.bold()).accessibilityAddTraits(.isHeader)
        ForEach(detail.related + detail.similar) { item in
          Button(item.label) { model.invoke(item.token) }.padding(.vertical, 6)
        }
        if !detail.message.isEmpty { Text(detail.message).font(.footnote).foregroundStyle(model.theme.muted) }
      }.frame(maxWidth: .infinity, alignment: .leading)
    }
  }
}
struct TrackMetadataContent: View {
  let detail: Details
  var body: some View {
    MetadataRows(rows: detail.rows, links: detail.links)
    if !detail.text.isEmpty { Text(detail.text).textSelection(.enabled) }
    if detail.loading { ProgressView() }
    if let lyrics = detail.lyrics {
      Text("Lyrics").font(.title3.bold())
      Text(lyrics).textSelection(.enabled)
    }
    RelatedMusicSection(detail: detail)
  }
}
struct DetailScreen: View {
  @Environment(AppModel.self) var model
  var body: some View {
    GeometryReader { geometry in
      ScrollView {
        if let detail = model.snapshot?.detail {
          VStack(alignment: .leading, spacing: 20) {
            if let artwork = detail.artwork { Artwork(id: artwork)
              .frame(
                width: min(340, max(0, geometry.size.width - 40)),
                height: min(340, max(0, geometry.size.width - 40))
              )
              .clipShape(RoundedRectangle(cornerRadius: 12)).frame(maxWidth: .infinity)
              .accessibilityIdentifier("track-detail-artwork")
            }
            VStack(alignment: .leading, spacing: 4) {
              Text(detail.title).font(.title2.bold()).textSelection(.enabled)
              if detail.kind == "album", !detail.subtitle.isEmpty {
                if let artist = detail.albumArtist {
                  Button(artist.label) { model.invoke(artist.token) }
                } else {
                  Text(detail.subtitle).foregroundStyle(model.theme.muted)
                }
              }
            }
            ActionButtons(
              actions: detail.actions, compact: detail.kind == "artist",
              showAll: detail.kind == "artist"
            ).frame(maxWidth: .infinity)
            TrackMetadataContent(detail: detail)
          }.padding(20).frame(maxWidth: .infinity, alignment: .leading)
        } else {
          ProgressView().frame(maxWidth: .infinity, minHeight: geometry.size.height)
        }
      }.frame(width: geometry.size.width, height: geometry.size.height)
        .accessibilityIdentifier("detail-content")
    }.background(model.theme.baseFill).navigationTitle("").navigationBarTitleDisplayMode(.inline)
  }
}
struct ActionButtons: View {
  @Environment(AppModel.self) var model
  let actions: [CommandButton]
  var compact = false
  var showAll = false
  var body: some View {
    if !actions.isEmpty {
      let primary = actions.enumerated().filter { index, action in
        showAll || (index < 3 && action.symbol != "text.line.first.and.arrowtriangle.forward")
          || action.symbol == "square.and.arrow.down"
          || (!compact && (action.symbol == "arrow.down.circle" || action.symbol == "trash"))
      }.map(\.element)
      if compact {
        HStack(spacing: 0) { ForEach(primary) { action in button(action) } }.fixedSize()
      } else {
        ViewThatFits(in: .horizontal) {
        HStack { ForEach(primary) { action in button(action) } }
        VStack(alignment: .leading) {
          ForEach(primary) { action in button(action) }
        }
      }
      }
      let primaryIds = Set(primary.map(\.id))
      let additional = actions.filter {
        !primaryIds.contains($0.id) && $0.symbol != "text.line.first.and.arrowtriangle.forward"
      }
      if !additional.isEmpty {
        Menu {
          let sections = additional.menuSections
          ForEach(sections.indices, id: \.self) { index in
            Section {
              ForEach(sections[index]) { action in
                Button(action.label) { model.invoke(action.token) }.disabled(action.disabled != nil)
              }
            }
          }
        } label: {
          if compact { LibraryControlIcon(symbol: "ellipsis.circle") }
          else { Image(systemName: "ellipsis.circle") }
        }.accessibilityLabel("More")
      }
    }
  }
  @ViewBuilder private func button(_ action: CommandButton) -> some View {
    if compact {
      actionButton(action).buttonStyle(.plain).foregroundStyle(model.theme.tint)
    } else {
      actionButton(action).buttonStyle(.borderedProminent).controlSize(.large)
        .foregroundStyle(model.theme.onTint)
    }
  }
  private func actionButton(_ action: CommandButton) -> some View {
    Button {
      model.invoke(action.token)
    } label: {
      if let mark = action.mark {
        Text(mark).font(.system(size: mark.count > 1 ? 14 : 22, weight: .bold))
          .frame(width: 48, height: 48).contentShape(Rectangle())
      } else if let symbol = action.symbol {
        if compact {
          LibraryControlIcon(symbol: symbol)
        } else {
          Image(systemName: symbol).font(.title3).frame(minWidth: 24, minHeight: 24)
        }
      } else {
        Text(action.label)
      }
    }.accessibilityLabel(action.label).disabled(action.disabled != nil)
      .contextMenu {
        ForEach(action.context ?? []) { alternate in
          Button(alternate.label) { model.invoke(alternate.token) }.disabled(alternate.disabled != nil)
        }
        if action.symbol == "play.fill" {
          Button(action.label) { model.invoke(action.token) }
          if let next = actions.first(where: {
            $0.symbol == "text.line.first.and.arrowtriangle.forward"
          }) {
            Button(next.label) { model.invoke(next.token) }.disabled(next.disabled != nil)
            ForEach(next.context ?? []) { alternate in
              Button(alternate.label) { model.invoke(alternate.token) }.disabled(alternate.disabled != nil)
            }
          }
        }
      }
  }
}
struct MetadataRows: View {
  @Environment(AppModel.self) var model
  let rows: [[String]]
  var links: [CommandButton] = []
  var body: some View {
    Grid(alignment: .leading, horizontalSpacing: 18, verticalSpacing: 9) {
      ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
        if row.count == 2 {
          GridRow {
            Text(row[0]).foregroundStyle(model.theme.muted)
            if let link = links.first(where: { $0.label == row[1] }) {
              Button(link.label) { model.invoke(link.token) }
            } else {
              Text(row[1]).textSelection(.enabled)
            }
          }
        }
      }
    }
    ForEach(links.filter { link in !rows.contains(where: { $0.last == link.label }) }) { link in
      Button(link.label) { model.invoke(link.token) }
    }
  }
}

/// Keep the navigation controller's interactive edge gesture with explicit back controls.
private struct LibrarySwipeBack: UIViewControllerRepresentable {
  final class Controller: UIViewController {
    override func viewDidAppear(_ animated: Bool) {
      super.viewDidAppear(animated)
      navigationController?.interactivePopGestureRecognizer?.delegate = nil
      navigationController?.interactivePopGestureRecognizer?.isEnabled = true
    }
  }
  func makeUIViewController(context: Context) -> Controller { Controller() }
  func updateUIViewController(_ controller: Controller, context: Context) {}
}

private struct LibraryNavigationControls: ViewModifier {
  @Environment(AppModel.self) var model
  func body(content: Content) -> some View {
    content
      .navigationBarBackButtonHidden(!model.path.isEmpty)
      .background(LibrarySwipeBack())
      .toolbar { LibraryBackControl() }
      .toolbar(model.path.isEmpty ? .hidden : .visible, for: .navigationBar)
  }
}

private struct LibraryBackControl: ToolbarContent {
  @Environment(AppModel.self) private var model
  var body: some ToolbarContent {
    if !model.path.isEmpty {
      ToolbarItem(placement: .topBarLeading) {
        Button {
          model.setPath(Array(model.path.dropLast()))
        } label: {
          LibraryControlIcon(symbol: "chevron.left")
        }
        .accessibilityLabel("Back")
        .contextMenu {
          Button("Up One Level", systemImage: "arrow.up") {
            model.setPath(Array(model.path.dropLast()))
          }
          Button("Return to Previous Location", systemImage: "arrow.uturn.backward") {
            model.send("return_to_previous_location")
          }.disabled(model.snapshot?.canReturn != true)
        }
      }
    }
  }
}
