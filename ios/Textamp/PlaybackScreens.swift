import SwiftUI

func timeLabel(_ milliseconds: Double) -> String {
  let seconds = max(0, Int(milliseconds / 1000))
  return seconds >= 3600
    ? String(format: "%d:%02d:%02d", seconds / 3600, (seconds / 60) % 60, seconds % 60)
    : String(format: "%d:%02d", seconds / 60, seconds % 60)
}
struct MiniPlayer: View {
  @Environment(AppModel.self) var model
  var body: some View {
    if let player = model.player {
      HStack(spacing: 12) {
        Button {
          model.selectTab("now_playing")
        } label: {
          HStack {
            Artwork(id: player.artwork).frame(width: 40, height: 40).playerBounds(.cover).clipShape(
              RoundedRectangle(cornerRadius: 5))
            VStack(alignment: .leading) {
              Text(player.title).font(.subheadline.bold()).lineLimit(1).playerBounds(.title)
              if !player.artist.isEmpty {
                Text(player.artist).font(.caption).foregroundStyle(model.theme.muted).lineLimit(1).playerBounds(.artist)
              }
              if player.autoMix == true {
                Label("Auto-Mix", systemImage: "link").font(.caption).foregroundStyle(
                  model.theme.muted)
              }
              if let mode = model.snapshot?.queueMode, mode != "Manual Queue" {
                Label(mode, systemImage: "dot.radiowaves.left.and.right").font(.caption2).lineLimit(
                  1
                )
                .foregroundStyle(model.theme.muted)
              }
            }
            Spacer()
          }.contentShape(Rectangle())
        }.buttonStyle(.plain).accessibilityLabel("Now Playing, \(player.title)")
        Button {
          model.transport("play_pause")
        } label: {
          Image(systemName: player.playing ? "pause.fill" : "play.fill").font(.title3).playerBounds(.play).frame(
            width: 44, height: 44)
        }.accessibilityLabel(player.playing ? "Pause" : "Play").disabled(!model.canPlay)
        Button {
          model.transport("next")
        } label: {
          Image(systemName: "forward.end.fill").font(.title3).playerBounds(.next).frame(width: 44, height: 44)
        }.accessibilityLabel("Next Track").disabled(!model.canPlay)
      }.foregroundStyle(model.theme.text).padding(.horizontal, 12).padding(.vertical, 8)
        .background(
          model.theme.system ? AnyShapeStyle(.regularMaterial) : AnyShapeStyle(model.theme.card),
          in: RoundedRectangle(cornerRadius: 22)
        )
        .overlay {
          RoundedRectangle(cornerRadius: 22).strokeBorder(
            model.theme.muted.opacity(0.16), lineWidth: 0.5)
        }
        .playerBounds(.surface)
        .padding(.horizontal, 12).padding(.vertical, 6)
        .accessibilityIdentifier("mini-player")

    }
  }
}
struct PlaybackSheetDetents: ViewModifier {
  @Binding var height: PresentationDetent
  let dismiss: () -> Void
  @State private var startFrame: CGRect?

  func body(content: Content) -> some View {
    GeometryReader { sheet in
      content.simultaneousGesture(
        DragGesture(minimumDistance: 16, coordinateSpace: .global)
          .onChanged { _ in
            if startFrame == nil { startFrame = sheet.frame(in: .global) }
          }
          .onEnded { drag in
            defer { startFrame = nil }
            guard let frame = startFrame,
                  drag.translation.height > abs(drag.translation.width) else { return }
            guard drag.startLocation.y < frame.minY + 80,
              drag.translation.height > frame.height * 0.64
                || drag.predictedEndTranslation.height > frame.height * 0.86 else { return }
            dismiss()
          })
    }
    .presentationDetents([.medium, .large], selection: $height)
    .presentationContentInteraction(.resizes)
    .presentationDragIndicator(.visible)
  }
}

enum PlayerPart: Hashable {
  case surface, cover, title, artist, play, next
}

@Observable final class PlayerLayout {
  var frames: [PlayerPart: CGRect] = [:]
}

private extension EnvironmentValues {
  @Entry var playerExpansion: CGFloat = 1
}

private struct EndpointInk: AnimatableModifier {
  var expansion: CGFloat
  var animatableData: CGFloat {
    get { expansion }
    set { expansion = newValue }
  }
  func body(content: Content) -> some View {
    content.opacity(max(0, 1 - (1 - expansion) * 60))
  }
}

private struct PlayerBoundsReader: ViewModifier {
  let part: PlayerPart
  @Environment(\.playerExpansion) private var expansion
  @Environment(PlayerLayout.self) private var layout

  func body(content: Content) -> some View {
    content.modifier(EndpointInk(expansion: expansion)).onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { frame in
      layout.frames[part] = frame
    }
  }
}

private extension View {
  func playerBounds(_ part: PlayerPart) -> some View {
    modifier(PlayerBoundsReader(part: part))
  }
}

struct TransportPresentation: View {
  let expanded: Bool
  let lift: CGFloat
  @Binding var presenting: Bool
  let transport: [PlayerPart: CGRect]
  let size: CGSize
  let origin: CGPoint
  @State private var fullLayout = PlayerLayout()
  @Environment(AppModel.self) private var model
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @State private var expansion: CGFloat = 0
  @GestureState private var dragging = false

  private func settle(open: Bool) {
    if open { presenting = true }
    withAnimation(reduceMotion ? .linear(duration: 0.15) : .smooth(duration: 0.42),
                  completionCriteria: .removed) {
      expansion = open ? 1 : 0
    } completion: {
      if !open && expansion == 0 && model.tab != "now_playing" { presenting = false }
    }
  }

  var body: some View {
    ZStack {
      if expanded || presenting {
        VStack(spacing: 0) {
          Button(action: model.dismissNowPlaying) {
            Capsule().fill(model.theme.muted).frame(width: 36, height: 5)
              .frame(maxWidth: .infinity).frame(height: 24).contentShape(Rectangle())
          }
          .buttonStyle(.plain).accessibilityLabel("Minimize Now Playing")
          NowPlayingNavigation()
        }
        .frame(width: size.width, height: size.height)
        .background(model.theme.bg)
        .environment(\.playerExpansion, reduceMotion ? 1 : expansion)
        .environment(fullLayout)
        .overlay {
          if let player = model.player {
            PlayerMorph(expansion: expansion, compact: transport,
              full: fullLayout.frames.mapValues { $0.offsetBy(dx: -origin.x, dy: -origin.y) },
              size: size, player: player, reduceMotion: reduceMotion)
              .allowsHitTesting(false).accessibilityHidden(true)
          }
        }
        .mask {
          if let bar = transport[.surface], !reduceMotion {
            RoundedRectangle(cornerRadius: 22)
              .frame(width: bar.width + expansion * (size.width - bar.width),
                     height: bar.height + expansion * (size.height - bar.height))
              .position(x: bar.midX + expansion * (size.width / 2 - bar.midX),
                        y: bar.midY + expansion * (size.height / 2 - bar.midY))
          } else { Rectangle() }
        }
        .opacity(reduceMotion ? expansion : 1)
        .contentShape(Rectangle())
        .simultaneousGesture(
          DragGesture(minimumDistance: 16, coordinateSpace: .global)
            .updating($dragging) { _, active, _ in active = true }
            .onChanged { value in
              guard value.translation.height > abs(value.translation.width) else { return }
              expansion = max(0, 1 - value.translation.height / max(1, size.height))
            }
            .onEnded { value in
              let down = value.translation.height
              if down > abs(value.translation.width), down > 40,
                 down > size.height * 0.35 || value.velocity.height > 520 {
                model.dismissNowPlaying()
              } else { settle(open: true) }
            })
        .accessibilityAddTraits(.isModal)
        .accessibilityAction(.escape, model.dismissNowPlaying)
      }
    }
    .onChange(of: lift) { _, distance in
      guard !expanded else { return }
      if distance > 0, let bar = transport[.surface] {
        presenting = true
        expansion = min(1, distance / max(1, bar.minY))
      } else if presenting {
        settle(open: model.tab == "now_playing")
      }
    }
    .onChange(of: expanded, initial: true) { _, open in settle(open: open) }
    .onChange(of: dragging) { _, active in
      if !active && model.tab == "now_playing" && expansion < 1 { settle(open: true) }
    }
  }
}

/// The moving cover, labels and controls use their actual bounds in both layouts.
private struct PlayerMorph: View, Animatable {
  var expansion: CGFloat
  let compact: [PlayerPart: CGRect]
  let full: [PlayerPart: CGRect]
  let size: CGSize
  let player: Player
  let reduceMotion: Bool
  @Environment(AppModel.self) private var model

  @ScaledMetric(relativeTo: .subheadline) private var subheadlineSize: CGFloat = 15
  @ScaledMetric(relativeTo: .title2) private var titleSize: CGFloat = 22
  @ScaledMetric(relativeTo: .caption) private var captionSize: CGFloat = 12

  var animatableData: CGFloat {
    get { expansion }
    set { expansion = newValue }
  }

  private func rect(_ part: PlayerPart) -> CGRect? {
    guard let a = compact[part], let b = full[part] else { return nil }
    func mix(_ x: CGFloat, _ y: CGFloat) -> CGFloat { x + expansion * (y - x) }
    return CGRect(x: mix(a.minX, b.minX), y: mix(a.minY, b.minY),
      width: mix(a.width, b.width), height: mix(a.height, b.height))
  }

  var body: some View {
    if !reduceMotion {
      ZStack(alignment: .topLeading) {
        model.theme.card.overlay { model.theme.bg.opacity(expansion) }
          .opacity(min(1, (1 - expansion) * 4))
        Group {
          if let cover = rect(.cover) {
            Artwork(id: player.artwork).frame(width: cover.width, height: cover.height)
              .clipShape(RoundedRectangle(cornerRadius: 5 + 9 * expansion))
              .position(x: cover.midX, y: cover.midY)
          }
          movingLabel(.title, text: player.title)
          movingLabel(.artist, text: player.artist)
          ForEach([PlayerPart.play, .next], id: \.self) { part in
            if let frame = rect(part) {
              Image(systemName: part == .next ? "forward.end.fill" : player.playing ? "pause.fill" : "play.fill")
                .font(.system(size: max(1, frame.height * 0.83)))
                .foregroundStyle(model.theme.text)
                .position(x: frame.midX, y: frame.midY)
            }
          }
        }
        .opacity(min(1, (1 - expansion) * 60))
      }
      .frame(width: size.width, height: size.height)
    }
  }

  // Keep each label typeset at its endpoint width so the animation cannot change its line breaks.
  @ViewBuilder private func movingLabel(_ part: PlayerPart, text: String) -> some View {
    if !text.isEmpty, let start = compact[part], let end = full[part], let frame = rect(part) {
      let isTitle = part == .title
      let small = isTitle ? subheadlineSize : captionSize
      let large = isTitle ? titleSize : subheadlineSize
      let pointSize = small + expansion * (large - small)
      let blend = min(1, max(0, (expansion - 0.35) / 0.3))
      ZStack {
        Text(text)
          .font(isTitle ? .subheadline.bold() : .caption)
          .lineLimit(1)
          .frame(width: start.width + 1)
          .scaleEffect(pointSize / small)
          .opacity(1 - blend)
        Text(text)
          .font(isTitle ? .title2.bold() : .subheadline)
          .multilineTextAlignment(.center)
          .frame(width: end.width + 1, height: end.height + 1)
          .scaleEffect(pointSize / large)
          .opacity(blend)
      }
      .foregroundStyle(isTitle ? model.theme.text : model.theme.muted)
      .position(x: frame.midX, y: frame.midY)
    }
  }

}

struct NowPlayingNavigation: View {
  @Environment(AppModel.self) var model
  var inWorkspace = false
  var body: some View {
    NavigationStack {
      NowPlayingScreen(inWorkspace: inWorkspace).toolbar {
        SheetDismissToolbarItem(label: "Close Now Playing") { model.dismissNowPlaying() }
      }
    }
  }
}
struct NowPlayingScreen: View {
  @Environment(AppModel.self) var model
  @Environment(\.dynamicTypeSize) private var dynamicTypeSize
  var inWorkspace = false
  @State private var waveformExpanded = false
  var body: some View {
    GeometryReader { geometry in
      if let player = model.player {
        VStack(spacing: 0) {
          GeometryReader { content in
            let available = max(0, content.size.width - 48)
            let horizontal =
              !dynamicTypeSize.isAccessibilitySize
              && ((inWorkspace && available >= 720)
                || (available >= 500 && content.size.width > content.size.height * 1.3))
            ScrollView {
              PlayerArrangement(
                availableHeight: max(0, content.size.height - 32), horizontal: horizontal
              ) {
                Artwork(id: player.artwork).aspectRatio(1, contentMode: .fit).playerBounds(.cover)
                  .clipShape(RoundedRectangle(cornerRadius: 14))
                  .shadow(color: .black.opacity(0.12), radius: 12, y: 5)
                controls(player)
              }
              .padding(.horizontal, 24).padding(.vertical, 16)
              .frame(maxWidth: horizontal ? 1040 : 580)
              .frame(maxWidth: .infinity)
              if inWorkspace { ActivityStatus() }
            }
          }
          volumeControls(player).padding(.horizontal, 24).padding(.vertical, 8)
            .background(model.theme.bg)
        }
      } else {
        ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
      }
    }.background(model.theme.bg)
      .accessibilityHidden(waveformExpanded)
      .fullScreenCover(isPresented: $waveformExpanded) {
        ExpandedPlaybackWaveform().environment(model)
      }
      .navigationTitle("Now Playing").navigationBarTitleDisplayMode(
        .inline
      )
  }
  private func controls(_ player: Player) -> some View {
    VStack(spacing: 12) {
      VStack(spacing: 0) {
        Button {
          model.send("metadata", ["target": "track"])
        } label: {
          Text(player.title).font(.title2.bold()).multilineTextAlignment(.center).playerBounds(.title).frame(
            minHeight: 44)
        }.foregroundStyle(model.theme.text).disabled(player.key == nil)
        let layout =
          dynamicTypeSize.isAccessibilitySize
          ? AnyLayout(VStackLayout(spacing: 0)) : AnyLayout(HStackLayout(spacing: 8))
        layout {
          Button {
            model.send("metadata", ["target": "artist"])
          } label: {
            Text(player.artist).lineLimit(dynamicTypeSize.isAccessibilitySize ? nil : 1).playerBounds(.artist)
              .frame(minHeight: 44)
          }
          if !dynamicTypeSize.isAccessibilitySize && player.key != nil {
            Text("·").accessibilityHidden(true)
          }
          Button {
            model.send("metadata", ["target": "album"])
          } label: {
            Text(player.album).lineLimit(dynamicTypeSize.isAccessibilitySize ? nil : 1).frame(
              minHeight: 44)
          }
        }.font(.subheadline).foregroundStyle(model.theme.muted)
        if let artist = player.albumArtist {
          Text("Album Artist: \(artist)").font(.subheadline).foregroundStyle(model.theme.muted)
        }
      }
      playbackMode(player)
      HStack(spacing: 8) {
        PlaybackWaveform(player: player)
        Button {
          waveformExpanded = true
        } label: {
          Image(systemName: "arrow.up.left.and.arrow.down.right")
            .font(.caption).padding(8).background(
              model.theme.card, in: RoundedRectangle(cornerRadius: 6))
        }.accessibilityLabel("Expand Waveform").frame(width: 44, height: 44)
          .disabled(player.key == nil)
      }
      PlaybackTransportControls(player: player)
    }
  }
  @ViewBuilder private func playbackMode(_ player: Player) -> some View {
    if player.autoMix == true {
      Label("Auto-Mix", systemImage: "link").font(.caption)
        .foregroundStyle(model.theme.muted)
    }
    if let mode = model.snapshot?.queueMode, mode != "Manual Queue" {
      Label(mode, systemImage: "dot.radiowaves.left.and.right").font(.caption)
        .foregroundStyle(model.theme.muted)
    }
  }
  private func volumeControls(_ player: Player) -> some View {
    HStack(spacing: 12) {
      Button {
        model.open("lyrics")
      } label: {
        Image(systemName: "quote.bubble").font(.title2).frame(width: 44, height: 44)
      }
      .accessibilityLabel("Lyrics").disabled(player.key == nil)
      Image(systemName: "speaker.fill")
      Slider(
        value: Binding(
          get: { player.volume }, set: { model.send("volume", ["value": $0]) }),
        in: 0...1
      ).accessibilityLabel("Volume")
      Image(systemName: "speaker.wave.3.fill")
    }.foregroundStyle(model.theme.muted)
  }
}
/// Reserve a square from the viewport so asynchronous playback updates cannot resize the cover.
private struct PlayerArrangement: Layout {
  let availableHeight: CGFloat
  let horizontal: Bool
  private func dimensions(_ width: CGFloat, _ subviews: Subviews) -> (CGFloat, CGSize, CGFloat) {
    let gap: CGFloat = horizontal ? 32 : 12
    let side =
      horizontal ? min(450, width * 0.45, availableHeight) : min(450, width, availableHeight * 0.48)
    let controls = subviews[1].sizeThatFits(
      .init(width: horizontal ? max(0, width - side - gap) : width, height: nil))
    return (side, controls, side > 0 ? gap : 0)
  }
  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    let width = proposal.width ?? 0
    let (side, controls, gap) = dimensions(width, subviews)
    return CGSize(
      width: width, height: horizontal ? max(side, controls.height) : side + gap + controls.height)
  }
  func placeSubviews(
    in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()
  ) {
    let (side, controls, gap) = dimensions(bounds.width, subviews)
    subviews[0].place(
      at: CGPoint(x: horizontal ? bounds.minX : bounds.midX - side / 2, y: bounds.minY),
      proposal: .init(width: side, height: side))
    subviews[1].place(
      at: CGPoint(
        x: horizontal ? bounds.minX + side + gap : bounds.minX,
        y: horizontal ? bounds.minY : bounds.minY + side + gap),
      proposal: .init(width: horizontal ? controls.width : bounds.width, height: controls.height))
  }
}

private struct PlaybackTransportControls: View {
  @Environment(AppModel.self) var model
  let player: Player
  var body: some View {
    HStack(spacing: 48) {
      Button {
        model.transport("previous")
      } label: {
        Image(systemName: "backward.end.fill").font(.system(size: 28)).frame(
          width: 52, height: 52)
      }.accessibilityLabel("Previous Track")
      Button {
        model.transport("play_pause")
      } label: {
        Image(systemName: player.playing ? "pause.fill" : "play.fill").font(
          .system(size: 42)
        )
        .playerBounds(.play).frame(width: 64, height: 64)
      }.accessibilityLabel(player.playing ? "Pause" : "Play")
        .accessibilityIdentifier("now-playing-toggle")
      Button {
        model.transport("next")
      } label: {
        Image(systemName: "forward.end.fill").font(.system(size: 28)).playerBounds(.next).frame(
          width: 52, height: 52)
      }.accessibilityLabel("Next Track")
    }.foregroundStyle(model.theme.text).disabled(!model.canPlay)
  }
}

struct QueueScreen: View {
  @Environment(AppModel.self) var model
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @State private var editMode = EditMode.inactive
  var docked = false
  var rows: [MusicRow] { model.snapshot?.queue ?? [] }
  var body: some View {
    List {
      ForEach(rows) { row in
        HStack(spacing: 10) {
          Button {
            model.send("queue_play", ["index": row.index, "key": row.key])
          } label: {
            Image(
              systemName: row.selected && model.player?.playing == true
                ? "speaker.wave.2.fill" : "play.fill"
            ).frame(width: 44, height: 44)
          }.buttonStyle(.borderless).accessibilityLabel("Play \(row.title)")
            .accessibilityValue(row.selected ? "Now Playing" : "")
          Button {
            Divider()
            if let details = row.details { model.invoke(details.token) }
          } label: {
            VStack(alignment: .leading, spacing: 5) {
              Text(row.title).lineLimit(2).foregroundStyle(model.theme.text)
              Text(row.subtitle).font(.subheadline).lineLimit(2).foregroundStyle(model.theme.muted)
            }.frame(maxWidth: .infinity, alignment: .leading)
          }.buttonStyle(.plain)
        }.padding(.vertical, 6).listRowBackground(
          row.selected ? model.theme.tint.opacity(0.16) : model.theme.card
        )
        .swipeActions {
          Button(role: .destructive) {
            model.send("queue_remove", ["index": row.index, "key": row.key])
          } label: {
            Label("Remove", systemImage: "trash")
          }
        }
        .contextMenu {
          Button("Play", systemImage: "play") {
            model.send("queue_play", ["index": row.index, "key": row.key])
          }
          if let details = row.details {
            Button("Track Details", systemImage: "info.circle") { model.invoke(details.token) }
          }
          if row.details != nil { Divider() }
          Button("Remove", systemImage: "trash", role: .destructive) {
            model.send("queue_remove", ["index": row.index, "key": row.key])
          }
        }
      }
      .onMove { offsets, to in
        guard let from = offsets.first else { return }
        move(from: from, to: to)
      }
      .onDelete { offsets in
        for index in offsets.sorted(by: >) {
          let row = rows[index]
          model.send("queue_remove", ["index": index, "key": row.key])
        }
      }
    }.environment(\.editMode, $editMode).scrollContentBackground(.hidden).background(model.theme.bg)
      .overlay {
        if rows.isEmpty {
          ContentUnavailableView(
            "Queue is empty", systemImage: "music.note.list",
            description: Text("Add music from your library."))
        }
      }
      .navigationTitle("Play Queue").navigationBarTitleDisplayMode(.inline)
      .toolbar {
        if !docked || editMode.isEditing {
          ToolbarItem(placement: .topBarLeading) {
            Button(editMode.isEditing ? "Done" : "Edit Queue",
                   systemImage: editMode.isEditing ? "checkmark" : "pencil", action: toggleEditing)
              .labelStyle(.iconOnly)
              .disabled(rows.isEmpty)
          }
        }
        ToolbarItemGroup(placement: .topBarTrailing) {
          queueType
          QueueToolsMenu(
            tools: model.snapshot?.tools ?? [], invoke: model.invoke,
            edit: docked && !editMode.isEditing ? toggleEditing : nil,
            canEdit: !rows.isEmpty
          ).equatable()
        }
      }
      .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: rows.map(\.id))
      .onChange(of: rows.isEmpty) { _, empty in if empty { editMode = .inactive } }
  }
  private func toggleEditing() {
    withAnimation(reduceMotion ? nil : .default) {
      editMode = editMode.isEditing ? .inactive : .active
    }
  }
  private var queueType: some View {
    Menu {
      ForEach(["Queue type", "Playback"], id: \.self) { group in
        Section(group) {
          ForEach((model.snapshot?.queueModes ?? []).filter { $0.group == group }) { choice in
            Button {
              if choice.active != true { model.invoke(choice.token) }
            } label: {
              if choice.active == true {
                Label(choice.label, systemImage: "checkmark")
              } else {
                Text(choice.label)
              }
            }.disabled(choice.disabled != nil)
          }
        }
      }
    } label: {
      Image(systemName: "list.bullet.rectangle").overlay(alignment: .topTrailing) {
        if let mode = model.snapshot?.queueMode, mode != "Manual Queue" {
          Circle().fill(model.theme.tint).frame(width: 6, height: 6).offset(x: 6, y: -4)
        }
      }
    }.accessibilityLabel("Queue Type")
      .accessibilityValue(model.snapshot?.queueMode ?? "Manual Queue")
  }
  private func move(from: Int, to: Int) {
    model.send("queue_move", ["from": from, "to": to, "keys": rows.map(\.key)])
  }
}
private struct QueueToolsMenu: View, Equatable {
  let tools: [CommandButton]
  let invoke: (UInt64) -> Void
  let edit: (() -> Void)?
  let canEdit: Bool
  static func == (lhs: Self, rhs: Self) -> Bool {
    lhs.tools == rhs.tools && lhs.canEdit == rhs.canEdit && (lhs.edit == nil) == (rhs.edit == nil)
  }
  var body: some View {
    Menu {
      if let edit {
        Button("Edit Queue", systemImage: "pencil", action: edit)
          .disabled(!canEdit)
        Divider()
      }
      let groups = tools.menuSections
      ForEach(groups.indices, id: \.self) { index in
        Section {
          ForEach(groups[index]) { tool in
        Button(role: tool.controlId == "queue-tool:ClearQueue" ? .destructive : nil) {
          invoke(tool.token)
        } label: {
          Label {
            Text(tool.label)
          } icon: {
            if tool.active == true {
              Image(systemName: "checkmark")
            } else {
              toolIcon(tool.controlId)
            }
          }
        }.disabled(tool.disabled != nil).accessibilityHint(tool.description ?? "")
          .accessibilityValue(tool.active == true ? "On" : "")
          }
        }
      }
    } label: {
      Image(systemName: "pencil.and.list.clipboard")
    }
    .accessibilityLabel("Queue Tools")
  }
}
private func toolIcon(_ identity: String?) -> Image {
  switch identity {
  case "queue-tool:Shuffle": return Image(systemName: "shuffle")
  case "queue-tool:Repeat": return Image(systemName: "repeat")
  case "queue-tool:Twins": return Image(systemName: "person.2")
  case "queue-tool:Twofer": return Image(systemName: "2.circle")
  case "queue-tool:Stretch": return Image(systemName: "arrow.left.and.right")
  case "queue-tool:Doppelganger": return Image(systemName: "theatermasks")
  case "queue-tool:OppositeDay": return Image("QueueOppositeDay")
  case "queue-tool:Enemies": return Image("QueueEnemies")
  case "queue-tool:ClearQueue": return Image(systemName: "trash")
  case "queue-tool:Save": return Image(systemName: "square.and.arrow.down")
  case "queue-tool:Undo": return Image(systemName: "arrow.uturn.backward")
  case "queue-tool:AutoMix": return Image(systemName: "link")
  default: return Image(systemName: "music.note.list")
  }
}

/// Keep the native tracking control mounted while waveform samples arrive behind it.
private struct PlaybackPositionSlider: UIViewRepresentable {
  @Environment(AppModel.self) var model
  let player: Player
  let identifier: String

  func makeCoordinator() -> Coordinator { Coordinator() }

  func makeUIView(context: Context) -> UISlider {
    let slider = UISlider()
    let thumb = UIGraphicsImageRenderer(size: CGSize(width: 12, height: 12)).image { context in
      UIColor.white.setFill()
      context.cgContext.fillEllipse(in: CGRect(x: 1, y: 1, width: 10, height: 10))
    }.withRenderingMode(.alwaysTemplate)
    slider.setThumbImage(thumb, for: .normal)
    slider.setThumbImage(thumb, for: .highlighted)
    slider.addTarget(
      context.coordinator, action: #selector(Coordinator.seek(_:)), for: .valueChanged)
    return slider
  }

  func updateUIView(_ slider: UISlider, context: Context) {
    slider.isEnabled = player.key != nil && player.duration > 0
    slider.accessibilityLabel = "Waveform Position"
    slider.accessibilityIdentifier = identifier
    slider.thumbTintColor = UIColor(model.theme.tint)
    let hasWaveform = player.waveform?.isEmpty == false
    slider.minimumTrackTintColor = hasWaveform ? .clear : UIColor(model.theme.tint)
    slider.maximumTrackTintColor = hasWaveform ? .clear : UIColor(model.theme.muted.opacity(0.35))
    if !slider.isTracking {
      slider.value = Float(max(0, min(1, player.position / max(1, player.duration))))
    }
    context.coordinator.commit = { fraction in
      guard player.key != nil else { return }
      model.send("seek", ["milliseconds": UInt64(max(0, min(1, fraction)) * player.duration)])
    }
  }

  final class Coordinator: NSObject {
    var commit: (Double) -> Void = { _ in }
    @objc func seek(_ slider: UISlider) { commit(Double(slider.value)) }
  }
}

private struct PlaybackTimeLabels: View {
  @Environment(AppModel.self) var model
  let player: Player
  var body: some View {
    HStack {
      Text(timeLabel(min(player.position, player.duration)))
      Spacer()
      Text("−" + timeLabel(max(0, player.duration - player.position)))
    }.foregroundStyle(model.theme.muted)
  }
}

struct PlaybackWaveform: View {
  @Environment(AppModel.self) var model
  let player: Player
  var expanded = false
  var height: CGFloat = 64
  var body: some View {
    VStack(spacing: 2) {
      GeometryReader { geometry in
        ZStack {
          Canvas { context, size in
            let bins = player.waveform ?? []
            for (index, amplitude) in bins.enumerated() {
              let x = CGFloat(index) / CGFloat(max(1, bins.count)) * size.width
              let height = max(1, CGFloat(amplitude) * size.height)
              let rect = CGRect(
                x: x, y: (size.height - height) / 2,
                width: max(1, size.width / CGFloat(max(1, bins.count)) - 1), height: height)
              context.fill(
                Path(rect),
                with: .color(
                  Double(index) / Double(max(1, bins.count)) <= player.position
                    / max(1, player.duration) ? model.theme.tint : model.theme.muted))
            }
          }.accessibilityElement(children: .ignore)
            .accessibilityIdentifier("playback-waveform")
            .accessibilityLabel("Waveform").accessibilityValue(
              player.waveformMessage ?? timeLabel(player.position)
            )
            .accessibilityAdjustableAction { direction in
              let delta: Double = direction == .increment ? 5000 : -5000
              model.send(
                "seek",
                ["milliseconds": UInt64(max(0, min(player.duration, player.position + delta)))])
            }
          PlaybackPositionSlider(
            player: player,
            identifier: expanded ? "expanded-waveform-position" : "waveform-position")
        }.contentShape(Rectangle())
          .simultaneousGesture(
            DragGesture(minimumDistance: 8).onEnded { value in
              guard abs(value.translation.width) > abs(value.translation.height) else { return }
              seek(to: value.location.x, width: geometry.size.width)
            }
          )
          .simultaneousGesture(
            SpatialTapGesture().onEnded { value in
              seek(to: value.location.x, width: geometry.size.width)
            }
          )

      }.frame(height: expanded ? nil : height)
      PlaybackTimeLabels(player: player).font(.caption2.monospacedDigit())
    }
  }
  private func seek(to x: CGFloat, width: CGFloat) {
    guard player.key != nil, width > 0, player.duration > 0 else { return }
    model.send("seek", ["milliseconds": UInt64(max(0, min(1, x / width)) * player.duration)])
  }

}

struct ExpandedPlaybackWaveform: View {
  @Environment(AppModel.self) var model
  @Environment(\.dismiss) private var dismiss
  var body: some View {
    VStack(spacing: 12) {
      HStack {
        Text(model.player?.title ?? "Now Playing").font(.headline).lineLimit(1)
        Spacer()
        SheetDismissButton { dismiss() }.padding(8)
      }
      if let player = model.player { PlaybackWaveform(player: player, expanded: true) }
    }.padding(16).frame(maxWidth: .infinity, maxHeight: .infinity)
      .foregroundStyle(model.theme.text).background(model.theme.bg.ignoresSafeArea())
      .background(WaveformOrientation())
  }
}

/// Request orientation after the full-screen host has completed presentation.
private struct WaveformOrientation: UIViewControllerRepresentable {
  final class Controller: UIViewController {
    private weak var scene: UIWindowScene?
    private var previous: UIInterfaceOrientationMask?
    override func viewDidAppear(_ animated: Bool) {
      super.viewDidAppear(animated)
      guard previous == nil, let scene = view.window?.windowScene else { return }
      self.scene = scene
      previous = scene.interfaceOrientation.isLandscape ? .landscape : .portrait
      scene.keyWindow?.rootViewController?.setNeedsUpdateOfSupportedInterfaceOrientations()
      scene.requestGeometryUpdate(.iOS(interfaceOrientations: .landscape))
    }
    override func viewDidDisappear(_ animated: Bool) {
      super.viewDidDisappear(animated)
      if let previous { scene?.requestGeometryUpdate(.iOS(interfaceOrientations: previous)) }
      previous = nil
    }
  }
  func makeUIViewController(context: Context) -> Controller {
    let controller = Controller()
    controller.view.isUserInteractionEnabled = false
    return controller
  }
  func updateUIViewController(_ controller: Controller, context: Context) {}
}
