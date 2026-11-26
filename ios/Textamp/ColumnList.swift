import SwiftUI
import UIKit

/// Center the header in the entire column while leaving the track list and its index beside the scrubber.
private final class AlbumTableHeader: UIView {
  init(content: UIView) {
    super.init(frame: .zero)
    content.translatesAutoresizingMaskIntoConstraints = false
    addSubview(content)
    insetsLayoutMarginsFromSafeArea = false
    NSLayoutConstraint.activate([
      content.leadingAnchor.constraint(equalTo: layoutMarginsGuide.leadingAnchor),
      content.trailingAnchor.constraint(equalTo: layoutMarginsGuide.trailingAnchor),
      content.topAnchor.constraint(equalTo: topAnchor),
      content.bottomAnchor.constraint(equalTo: bottomAnchor),
    ])
  }
  required init?(coder: NSCoder) { nil }
}
private final class PlaylistTable: UITableView {
  override func layoutSubviews() {
    super.layoutSubviews()
    if let view = tableHeaderView, bounds.width > 0 {
      let inset = max(0, (superview?.bounds.width ?? bounds.width) - bounds.width)
      let margins = NSDirectionalEdgeInsets(top: 0, leading: inset, bottom: 0, trailing: 0)
      if view.directionalLayoutMargins != margins { view.directionalLayoutMargins = margins }
      let size = view.systemLayoutSizeFitting(CGSize(width: bounds.width, height: 0),
        withHorizontalFittingPriority: .required, verticalFittingPriority: .fittingSizeLevel)
      let height = ceil(size.height)
      if view.frame.height != height || view.frame.width != bounds.width {
        view.frame.size = CGSize(width: bounds.width, height: height)
        tableHeaderView = view
      }
    }
  }
  var keyboardTarget = false
  override func didMoveToWindow() {
    super.didMoveToWindow()
    if window != nil && keyboardTarget { becomeFirstResponder() }
  }
  var supportsSelection = false
  var selectPlaylistRows: (() -> Void)?
  override var canBecomeFirstResponder: Bool { true }
  override var keyCommands: [UIKeyCommand]? {
    guard supportsSelection else { return super.keyCommands }
    return (super.keyCommands ?? []) + [UIKeyModifierFlags.command, .control].map {
      UIKeyCommand(input: "a", modifierFlags: $0, action: #selector(selectPlaylist))
    }
  }
  @objc private func selectPlaylist() { selectPlaylistRows?() }
}

/// A virtual native list keeps the scrollbar proportional to the entire column.
struct ColumnList: UIViewRepresentable {
  @Environment(AppModel.self) var model
  @Environment(\.colorScheme) private var colorScheme
  let column: Column
  let paneWidth: CGFloat
  let showsSelection: Bool
  let keyboardTarget: Bool
  let selecting: Bool
  let beginSelecting: () -> Void
  let showAlbumNotes: () -> Void
  var dismissFilterKeyboard: () -> Void

  private var artworkColumns: Int { max(2, Int((paneWidth - ColumnListContainer.scrubberWidth) / 130)) }

  func makeCoordinator() -> Coordinator { Coordinator(self) }
  func makeUIView(context: Context) -> ColumnListContainer {
    let table = PlaylistTable(frame: .zero, style: .plain)
    let coordinator = context.coordinator
    table.selectPlaylistRows = { [weak coordinator] in coordinator?.selectAllRows() }
    let click = UITapGestureRecognizer(target: coordinator, action: #selector(Coordinator.togglePointerRow(_:)))
    click.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.indirectPointer.rawValue)]
    click.delegate = coordinator
    table.addGestureRecognizer(click)
    table.dataSource = context.coordinator
    table.delegate = context.coordinator
    table.register(UITableViewCell.self, forCellReuseIdentifier: "music")
    table.selectionFollowsFocus = false
    table.allowsMultipleSelectionDuringEditing = true
    table.showsVerticalScrollIndicator = true
    table.contentInsetAdjustmentBehavior = .always
    table.keyboardDismissMode = .interactive
    table.alwaysBounceVertical = true
    table.sectionHeaderTopPadding = 0
    table.sectionIndexMinimumDisplayRowCount = 0
    table.accessibilityIdentifier = "library-column"
    return ColumnListContainer(table: table)
  }
  func updateUIView(_ container: ColumnListContainer, context: Context) {
    context.coordinator.update(self, table: container.table)
    container.setNeedsLayout()
  }
  static func dismantleUIView(_ table: ColumnListContainer, coordinator: Coordinator) {
    coordinator.removeIndexPreview()
  }

  final class Coordinator: NSObject, UITableViewDataSource, UITableViewDelegate, UIGestureRecognizerDelegate {
    private var parent: ColumnList
    private var rows: [Int: MusicRow] = [:]
    private var sections: [ColumnSection] = []
    private var indexedSections: [Int] = []
    private var requestedWindow: Range<Int>?
    private var requestScheduled = false
    private struct GestureMarks {
      let owner: String
      var rows: [Int: (key: String, selected: Bool)] = [:]
    }
    private var gesture: GestureMarks?
    private var initialized = false
    private var requestedArtwork: [String: String] = [:]
    private var theme = AppTheme()
    private var gridColumns = 1
    private var rowHeight: CGFloat = 76
    private let indexPreview = UILabel()
    private var hideIndexPreview: Task<Void, Never>?

    init(_ parent: ColumnList) { self.parent = parent }

    func selectAllRows() {
      parent.model.send("select_all", ["owner": parent.column.id])
    }
    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive event: UIEvent) -> Bool {
      (parent.column.playlistEditControls?.isEmpty == false || parent.column.rows.contains { $0.kind == "track" })
        && !event.modifierFlags.intersection([.command, .control]).isEmpty
    }
    @objc func togglePointerRow(_ gesture: UITapGestureRecognizer) {
      guard let table = gesture.view as? UITableView,
        let path = table.indexPathForRow(at: gesture.location(in: table)), let row = rows[position(path)],
        ["track", "album"].contains(row.kind) else { return }
      table.becomeFirstResponder()
      if parent.column.playlistEditControls?.isEmpty != false {
        parent.model.send("toggle_track_selection", ["owner": parent.column.id, "index": row.index, "key": row.key])
        return
      }
      parent.model.send("playlist_gesture", ["owner": parent.column.id,
        "marks": [["index": row.index, "key": row.key, "selected": row.batchSelected != true]]])
    }
    func removeIndexPreview() {
      hideIndexPreview?.cancel()
      indexPreview.removeFromSuperview()
    }

    func update(_ next: ColumnList, table: UITableView) {
      let old = parent.column
      let reset = !initialized || old.id != next.column.id || old.filter != next.column.filter
      if old.id != next.column.id { requestedArtwork.removeAll() }
      let height = UIFontMetrics.default.scaledValue(for: 76)
      let relatedChanged = old.albumPage?.similar != next.column.albumPage?.similar
        || old.relatedArtists != next.column.relatedArtists
        || old.albumPage?.message != next.column.albumPage?.message
      let columns = next.column.gridStart != nil && !next.selecting && next.column.playlistEditing != true
        ? next.artworkColumns : 1
      let structureChanged =
        reset || gridColumns != columns || old.gridStart != next.column.gridStart || old.gridEnd != next.column.gridEnd || old.showsRecommendations != next.column.showsRecommendations || old.count != next.column.count || old.sections != next.column.sections
        || height != rowHeight
      let editing = next.selecting || next.column.playlistEditing == true
      let contentChanged =
        parent.selecting != next.selecting || old.playlistEditing != next.column.playlistEditing || old.rows != next.column.rows
        || parent.showsSelection != next.showsSelection
        || theme != next.model.theme
        || parent.colorScheme != next.colorScheme
      let album = next.column.albumArtworkKey
      let document = next.column.albumPage
      if reset || (document.map { !$0.hasSameAlbumHeader(as: old.albumPage) } ?? (old.albumPage != nil))
        || old.artistHeader != next.column.artistHeader || old.playback != next.column.playback || old.albumDetailsArtwork != next.column.albumDetailsArtwork
        || theme != next.model.theme {
        if album != nil {
          table.tableHeaderView = AlbumTableHeader(content: UIHostingConfiguration {
            AlbumPageHeader(column: next.column, detail: document, showAlbumNotes: next.showAlbumNotes).environment(next.model)
              .foregroundStyle(next.model.theme.text).tint(next.model.theme.tint)
          }.margins(.all, 0).makeContentView())
        } else if let info = next.column.artistHeader {
          table.tableHeaderView = AlbumTableHeader(content: UIHostingConfiguration {
            VStack(spacing: 14) {
              Color.clear.aspectRatio(1, contentMode: .fit)
                .frame(maxWidth: 220)
                .overlay { Artwork(id: info.artwork) }
                .clipShape(Circle())
                .frame(maxWidth: .infinity, alignment: .center)
              if let search = info.search { ArtistWebSearchButton(command: search, showsLabel: true) } else {
              VStack(alignment: .trailing, spacing: 0) {
                Text(info.text).font(.body).lineLimit(3).frame(maxWidth: .infinity, alignment: .leading)
                  .accessibilityIdentifier("artist-bio-preview")
                Button(info.loading == true ? "Artist Biography" : "…more") { next.model.invoke(info.reader.token) }
                  .font(.subheadline.weight(.semibold)).accessibilityLabel(info.reader.label)
              }
              }
            }.frame(maxWidth: .infinity).padding().foregroundStyle(next.model.theme.text).tint(next.model.theme.tint).environment(next.model)
          }.margins(.all, 0).makeContentView())
        } else { table.tableHeaderView = nil }

      }
      let reflow = initialized && !reset && columns != gridColumns
      let anchor = reflow ? table.indexPathsForVisibleRows?.first(where: { $0.section < sections.count }) : nil
      let anchorPosition = anchor.map { position($0) }
      let anchorOffset = anchor.map { table.rectForRow(at: $0).minY - table.contentOffset.y }
      gridColumns = columns
      theme = next.model.theme
      rowHeight = height
      if structureChanged { gesture = nil }
      parent = next
      if let list = table as? PlaylistTable {
        list.supportsSelection = editing || next.column.rows.contains { $0.kind == "track" }
        let target = next.keyboardTarget && list.supportsSelection
        if target && !list.keyboardTarget && list.window != nil { list.becomeFirstResponder() }
        list.keyboardTarget = target
      }
      if gesture == nil && table.isEditing != editing {
        table.setEditing(editing, animated: false)
        if editing { table.becomeFirstResponder() }
      }
      table.tintColor = UIColor(next.model.theme.tint)
      table.backgroundColor = next.model.theme.tableBackground
      table.separatorColor = UIColor(next.model.theme.muted.opacity(0.2))
      table.sectionIndexColor = UIColor(next.model.theme.tint)
      table.sectionIndexBackgroundColor = .clear
      table.indicatorStyle = next.colorScheme == .dark ? .white : .black
      if !initialized || contentChanged || structureChanged {
        rows = Dictionary(
          uniqueKeysWithValues: next.column.rows.map { ($0.position ?? $0.index, $0) })
        if structureChanged {
          sections =
            next.column.sections.flatMap { $0.isEmpty ? nil : $0 } ?? [
              ColumnSection(label: "", position: 0)
            ]
          indexedSections = sections.indices.filter { !sections[$0].label.isEmpty }
            .sorted { sections[$0].label < sections[$1].label }

          requestedWindow = nil
          let reload = {
            for cell in table.visibleCells {
              cell.contentConfiguration = nil
              cell.accessibilityIdentifier = nil
            }
            table.reloadData()
            table.layoutIfNeeded()
            if let position = anchorPosition, let offset = anchorOffset, position < next.column.count {
              let y = table.rectForRow(at: self.indexPath(position)).minY - offset
              let top = -table.adjustedContentInset.top
              let bottom = max(top, table.contentSize.height - table.bounds.height + table.adjustedContentInset.bottom)
              table.setContentOffset(CGPoint(x: 0, y: min(bottom, max(top, y))), animated: false)
            }
          }
          if reflow && table.window != nil && !UIAccessibility.isReduceMotionEnabled {
            UIView.transition(with: table, duration: 0.16, options: [.transitionCrossDissolve, .beginFromCurrentState, .allowUserInteraction], animations: reload)
          } else { reload() }
          if reset, next.column.count > 0 {
            let target = old.filter != next.column.filter ? 0 : next.column.selectedPosition ?? 0
            if (album != nil || next.column.artistHeader != nil) && target == 0 {
              table.setContentOffset(CGPoint(x: 0, y: -table.adjustedContentInset.top), animated: false)
            } else {
              table.scrollToRow(at: indexPath(min(target, next.column.count - 1)), at: .top, animated: false)
            }
          }
        } else {
          for path in table.indexPathsForVisibleRows ?? [] {
            if let cell = table.cellForRow(at: path) { configure(cell, at: path) }
          }
        }
      }
      if relatedChanged && !structureChanged && !contentChanged && parent.column.showsRecommendations {
        table.reconfigureRows(at: [IndexPath(row: 0, section: sections.count)])
      }
      if editing && gesture == nil {
        for path in table.indexPathsForVisibleRows ?? [] {
          guard let row = rows[position(path)] else { continue }
          if row.batchSelected == true {
            table.selectRow(at: path, animated: false, scrollPosition: .none)
          } else {
            table.deselectRow(at: path, animated: false)
          }
        }
      }
      initialized = true
      scheduleRequest(table)
    }

    func numberOfSections(in tableView: UITableView) -> Int { sections.count + (parent.column.showsRecommendations ? 1 : 0) }
    func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
      if section == sections.count { return 1 }
      let end = section + 1 < sections.count ? sections[section + 1].position : parent.column.count
      let grid = gridRange(section)
      return end - sections[section].position - grid.count + (grid.count + gridColumns - 1) / gridColumns
    }
    func tableView(_ tableView: UITableView, cellForRowAt path: IndexPath) -> UITableViewCell {
      let cell = tableView.dequeueReusableCell(withIdentifier: "music", for: path)
      configure(cell, at: path)
      return cell
    }
    private func gridRange(_ section: Int) -> Range<Int> {
      let start = sections[section].position
      let end = section + 1 < sections.count ? sections[section + 1].position : parent.column.count
      guard gridColumns > 1, let first = parent.column.gridStart, let last = parent.column.gridEnd else { return start..<start }
      let lower = min(end, max(start, first))
      return lower..<max(lower, min(last, end))
    }
    private func position(_ path: IndexPath) -> Int {
      guard path.section < sections.count else { return parent.column.count }
      let raw = sections[path.section].position + path.row
      let grid = gridRange(path.section)
      guard raw >= grid.lowerBound else { return raw }
      let band = raw - grid.lowerBound
      let bands = (grid.count + gridColumns - 1) / gridColumns
      return band < bands ? grid.lowerBound + band * gridColumns : grid.upperBound + band - bands
    }
    private func indexPath(_ position: Int) -> IndexPath {
      let section = sections.lastIndex(where: { $0.position <= position }) ?? 0
      let grid = gridRange(section)
      var row = position - sections[section].position
      if grid.contains(position) { row -= (position - grid.lowerBound) - (position - grid.lowerBound) / gridColumns }
      else if position >= grid.upperBound { row -= grid.count - (grid.count + gridColumns - 1) / gridColumns }
      return IndexPath(row: row, section: section)
    }
    private func configure(_ cell: UITableViewCell, at path: IndexPath) {
      if path.section == sections.count {
        cell.contentView.isUserInteractionEnabled = true
        let columns = parent.artworkColumns
        cell.contentConfiguration = UIHostingConfiguration {
          ColumnRecommendations(columns: columns, albums: parent.column.albumPage?.similar ?? [], artists: parent.column.relatedArtists, message: parent.column.albumPage?.message ?? "").environment(parent.model)
        }.margins(.horizontal, 8).margins(.vertical, 12)
        cell.backgroundColor = parent.model.theme.isBrushedAluminum ? .clear : UIColor(parent.model.theme.bg)
        cell.accessoryType = .none
        cell.editingAccessoryType = .none
        cell.accessibilityIdentifier = "album-recommendations"
        cell.accessibilityTraits.remove(.selected)
        return
      }
      let start = position(path)
      let range = gridRange(path.section)
      if range.contains(start) {
        cell.contentView.isUserInteractionEnabled = true
        cell.backgroundColor = parent.model.theme.isBrushedAluminum ? .clear : UIColor(parent.model.theme.bg)
        cell.accessoryType = .none
        cell.editingAccessoryType = .none
        cell.accessibilityIdentifier = nil
        cell.accessibilityTraits.remove(.selected)
        cell.selectionStyle = .none
        let column = parent.column
        let model = parent.model
        let tiles = (0..<gridColumns).map { start + $0 < range.upperBound ? rows[start + $0] : nil }
        cell.contentConfiguration = UIHostingConfiguration {
          HStack(alignment: .top, spacing: 10) {
            ForEach(tiles.indices, id: \.self) { slot in
              if let row = tiles[slot] {
                Button {
                  model.send("select", ["owner": column.id, "index": row.index, "key": row.key])
                } label: {
                  VStack(alignment: .leading, spacing: 4) {
                    Color.clear.aspectRatio(1, contentMode: .fit).overlay { Artwork(id: row.artwork) }.clipShape(RoundedRectangle(cornerRadius: 6))
                    ViewThatFits(in: .horizontal) {
                      Text(row.albumCaption).fixedSize()
                      if let balanced = row.balancedCaption { Text(balanced).fixedSize() }
                      Text(row.albumCaption).lineLimit(3)
                    }.font(.subheadline)
                    if column.artistHeader == nil, !row.subtitle.isEmpty { Text(row.subtitle).font(.caption).lineLimit(1) }
                  }.frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                }.buttonStyle(.plain).accessibilityLabel(row.albumCaption).contextMenu {
                  ForEach(row.context ?? []) { command in
                    Button(command.label) { model.invoke(command.token) }.disabled(command.disabled != nil)
                  }
                }
              } else { Color.clear.frame(maxWidth: .infinity) }
            }
          }.foregroundStyle(model.theme.text).environment(model)
        }.margins(.horizontal, 8).margins(.vertical, 5)
        return
      }
      let editing = parent.selecting || parent.column.playlistEditing == true
      let marked = rows[position(path)]?.batchSelected == true
      cell.tintColor = UIColor(
        editing ? parent.model.theme.tint : parent.model.theme.onTint)
      cell.editingAccessoryType =
        parent.column.playlistEditing == true && rows[position(path)]?.kind == "album"
        ? .detailButton : .none
      cell.accessoryType = marked && !editing ? .checkmark : .none
      let selected =
        !editing
        && (marked || (parent.showsSelection && rows[position(path)]?.selected == true))
      cell.backgroundColor =
        selected ? UIColor(parent.model.theme.tint) : UIColor(parent.model.theme.card)
      if marked || selected {
        cell.accessibilityTraits.insert(.selected)
      } else {
        cell.accessibilityTraits.remove(.selected)
      }
      cell.selectionStyle = .default
      let highlight = cell.selectedBackgroundView ?? UIView()
      highlight.backgroundColor = UIColor(parent.model.theme.tint.opacity(0.18))
      cell.selectedBackgroundView = highlight
      if let row = rows[position(path)] {
        // UIKit owns row taps and pans. Only pending downloads embed a button.
        cell.contentView.isUserInteractionEnabled = row.cancel != nil
        cell.accessibilityIdentifier = "column-row-\(row.key)"
        cell.contentConfiguration = UIHostingConfiguration {
          MusicRowView(
            row: row,
            selected: selected
          )
          .environment(parent.model)
        }.margins(.horizontal, 18).margins(.vertical, 7)
      } else {
        cell.accessibilityIdentifier = nil
        cell.contentConfiguration = UIHostingConfiguration {
          ProgressView().frame(maxWidth: .infinity, alignment: .leading)
        }.margins(.all, 18)
      }
    }
    func tableView(_ tableView: UITableView, heightForRowAt path: IndexPath) -> CGFloat {
      if path.section == sections.count { return UITableView.automaticDimension }
      if gridRange(path.section).contains(position(path)) {
        return max(1, tableView.bounds.width / CGFloat(gridColumns)) + UIFontMetrics.default.scaledValue(for: 82)
      }
      return rowHeight
    }
    func tableView(_ tableView: UITableView, didSelectRowAt path: IndexPath) {
      if path.section == sections.count || gridRange(path.section).contains(position(path)) {
        tableView.deselectRow(at: path, animated: true)
        return
      }
      if tableView.isEditing {
        mark(path, selected: true)
        return
      }
      tableView.deselectRow(at: path, animated: true)
      guard let row = rows[position(path)], row.pending == nil else { return }
      if let token = row.activation {
        parent.model.invoke(token)
      } else {
        parent.model.send(
          "select", ["owner": parent.column.id, "index": row.index, "key": row.key])
      }
    }
    func tableView(_ tableView: UITableView, accessoryButtonTappedForRowWith path: IndexPath) {
      guard let row = rows[position(path)], row.kind == "album" else { return }
      parent.model.send("select", ["owner": parent.column.id, "index": row.index, "key": row.key])
    }
    private func mark(_ path: IndexPath, selected: Bool) {
      guard let row = rows[position(path)], row.batchSelected != nil, row.pending == nil else { return }
      if gesture != nil {
        gesture?.rows[row.index] = (row.key, selected)
        return
      }
      parent.model.send(
        "playlist_mark",
        ["owner": parent.column.id, "index": row.index, "key": row.key, "selected": selected])
    }
    func tableView(_ tableView: UITableView, didDeselectRowAt path: IndexPath) {
      if tableView.isEditing { mark(path, selected: false) }
    }
    func tableView(
      _ tableView: UITableView, shouldBeginMultipleSelectionInteractionAt path: IndexPath
    ) -> Bool {
      guard let row = rows[position(path)] else { return false }
      return row.batchSelected != nil && row.pending == nil
    }
    func tableView(_ tableView: UITableView, didBeginMultipleSelectionInteractionAt path: IndexPath)
    {
      gesture = GestureMarks(owner: parent.column.id)
      tableView.setEditing(true, animated: true)
      if parent.column.playlistEditControls?.isEmpty != false { parent.beginSelecting() }
      if let row = rows[position(path)] {
        gesture?.rows[row.index] = (row.key, row.batchSelected != true)
      }
    }
    func tableViewDidEndMultipleSelectionInteraction(_ tableView: UITableView) {
      guard var finished = gesture else { return }
      gesture = nil
      let selected = Set(tableView.indexPathsForSelectedRows ?? [])
      for path in tableView.indexPathsForVisibleRows ?? [] {
        guard let row = rows[position(path)], row.batchSelected != nil, row.pending == nil,
          selected.contains(path), row.batchSelected != true
        else { continue }
        finished.rows[row.index] = (row.key, selected.contains(path))
      }
      let marks: [[String: Any]] = finished.rows.sorted { $0.key < $1.key }.map { index, value in
        ["index": index, "key": value.key, "selected": value.selected]
      }
      parent.model.send("playlist_gesture", ["owner": finished.owner, "marks": marks])
    }

    func tableView(
      _ tableView: UITableView, trailingSwipeActionsConfigurationForRowAt path: IndexPath
    ) -> UISwipeActionsConfiguration? {
      guard path.section < sections.count,
        let remove = rows[position(path)]?.context?.first(where: { $0.kind == "removeDescription" })
      else { return nil }
      let model = parent.model
      let action = UIContextualAction(style: .destructive, title: remove.label) { _, _, complete in
        model.invoke(remove.token)
        complete(true)
      }
      action.image = UIImage(systemName: "trash")
      return UISwipeActionsConfiguration(actions: [action])
    }
    func tableView(
      _ tableView: UITableView, contextMenuConfigurationForRowAt path: IndexPath, point: CGPoint
    ) -> UIContextMenuConfiguration? {
      guard path.section < sections.count, !gridRange(path.section).contains(position(path)), let commands = rows[position(path)]?.context, !commands.isEmpty else { return nil }
      let model = parent.model
      return UIContextMenuConfiguration(identifier: nil, previewProvider: nil) { _ in
        UIMenu(
          children: commands.menuSections.map { section in
            UIMenu(
              options: .displayInline,
              children: section.map { command in
                var attributes: UIMenuElement.Attributes = command.destructive == true ? .destructive : []
                if command.disabled != nil { attributes.insert(.disabled) }
                return UIAction(title: command.label, attributes: attributes)
                {
                  _ in model.invoke(command.token)
                }
              })
          })
      }
    }
    func sectionIndexTitles(for tableView: UITableView) -> [String]? {
      indexedSections.isEmpty ? nil : indexedSections.map { sections[$0].label }
    }
    func tableView(
      _ tableView: UITableView, sectionForSectionIndexTitle title: String, at index: Int
    ) -> Int {
      showIndexPreview(title, in: tableView)
      return indexedSections[index]
    }
    private func showIndexPreview(_ title: String, in table: UITableView) {
      guard let container = table.superview else { return }
      if indexPreview.superview !== container {
        indexPreview.removeFromSuperview()
        container.addSubview(indexPreview)
      }
      if indexPreview.isHidden || indexPreview.text != title {
        UISelectionFeedbackGenerator().selectionChanged()
      }
      indexPreview.text = title
      indexPreview.font = .systemFont(ofSize: 48, weight: .semibold)
      indexPreview.textAlignment = .center
      indexPreview.textColor = UIColor(parent.model.theme.tint)
      indexPreview.backgroundColor = UIColor(parent.model.theme.card)
      indexPreview.layer.cornerRadius = 18
      indexPreview.layer.borderWidth = 1
      indexPreview.layer.borderColor = UIColor(parent.model.theme.muted.opacity(0.25)).cgColor
      indexPreview.clipsToBounds = true
      indexPreview.isUserInteractionEnabled = false
      indexPreview.accessibilityIdentifier = "alphabet-preview"
      indexPreview.frame = CGRect(
        x: table.frame.maxX - 112, y: table.frame.midY - 40, width: 80, height: 80)
      indexPreview.isHidden = false
      container.bringSubviewToFront(indexPreview)
      hideIndexPreview?.cancel()
      hideIndexPreview = Task { @MainActor [weak self] in
        do { try await Task.sleep(for: .milliseconds(700)) } catch { return }
        self?.indexPreview.isHidden = true
      }
    }
    func tableView(
      _ tableView: UITableView, willDisplay cell: UITableViewCell, forRowAt path: IndexPath
    ) {
      if tableView.isEditing && gesture == nil
        && rows[position(path)]?.batchSelected == true
      {
        tableView.selectRow(at: path, animated: false, scrollPosition: .none)
      }
      scheduleRequest(tableView)
    }
    func scrollViewDidScroll(_ scrollView: UIScrollView) {
      if let table = scrollView as? UITableView {
        (table.superview as? ColumnListContainer)?.updateThumb()
        scheduleRequest(table)
      }
    }
    func scrollViewWillBeginDragging(_ scrollView: UIScrollView) {
      if scrollView.panGestureRecognizer.velocity(in: scrollView).y > 0 {
        // The filter field belongs to the toolbar, outside the table's responder subtree.
        parent.dismissFilterKeyboard()
      }
    }
    private func scheduleRequest(_ table: UITableView) {
      guard !requestScheduled else { return }
      requestScheduled = true
      DispatchQueue.main.async { [weak self, weak table] in
        guard let self, let table else { return }
        self.requestScheduled = false
        self.requestVisible(table)
      }
    }
    private func requestVisible(_ table: UITableView) {
      let visible = table.indexPathsForVisibleRows ?? []
      let positions = visible.filter { $0.section < sections.count }.flatMap { path -> Range<Int> in
        let start = position(path)
        let grid = gridRange(path.section)
        return start..<(grid.contains(start) ? min(grid.upperBound, start + gridColumns) : start + 1)
      }
      guard let start = positions.min(), let last = positions.max() else { return }
      if positions.contains(where: { rows[$0] == nil }), requestedWindow != start..<(last + 1) {
        requestedWindow = start..<(last + 1)
        parent.model.send("viewport", ["owner": parent.column.id, "start": start, "end": last + 1])
      }
      let artworkRows = positions.compactMap { rows[$0] }.filter { $0.artwork != nil }
      let visibleKeys = Set(artworkRows.map(\.key))
      requestedArtwork = requestedArtwork.filter { visibleKeys.contains($0.key) }
      let keys = artworkRows.compactMap { row -> String? in
        guard let artwork = row.artwork, requestedArtwork[row.key] != artwork else { return nil }
        requestedArtwork[row.key] = artwork
        return row.key
      }
      if !keys.isEmpty {
        parent.model.send("artwork", ["owner": parent.column.id, "keys": keys])
      }
      if parent.column.more == true && last >= parent.column.count - 5 {
        parent.model.send("more_rows", ["owner": parent.column.id])
      }
    }
  }
}

/// A separate scrubber keeps the table's alphabet index and proportional scrolling independently hittable.
final class ColumnListContainer: UIView {
  static let scrubberWidth: CGFloat = 28
  let table: UITableView
  private let scrubber = ListScrubber()
  init(table: UITableView) {
    self.table = table
    super.init(frame: .zero)
    addSubview(table)
    addSubview(scrubber)
    scrubber.table = table
  }
  required init?(coder: NSCoder) { nil }
  override func layoutSubviews() {
    super.layoutSubviews()
    let show = table.contentSize.height > bounds.height * 1.5 && bounds.height > 0
    let railWidth = show ? Self.scrubberWidth : 0
    let leading = effectiveUserInterfaceLayoutDirection == .rightToLeft
    table.frame = CGRect(
      x: leading ? railWidth : 0, y: 0, width: max(0, bounds.width - railWidth),
      height: bounds.height)
    scrubber.frame = CGRect(
      x: leading ? 0 : table.frame.maxX, y: 0, width: railWidth, height: bounds.height)
    scrubber.isHidden = !show
    table.showsVerticalScrollIndicator = !show
    backgroundColor = table.backgroundColor
    updateThumb()
  }
  func updateThumb() { scrubber.setNeedsLayout() }
}

private final class ListScrubber: UIControl {
  weak var table: UITableView?
  private let thumb = UIView()
  private var grabOffset: CGFloat = 0
  override init(frame: CGRect) {
    super.init(frame: frame)
    isAccessibilityElement = true
    accessibilityTraits = .adjustable
    accessibilityLabel = "Scroll position"
    accessibilityIdentifier = "column-scrollbar"
    thumb.isUserInteractionEnabled = false
    addSubview(thumb)
    let hold = UILongPressGestureRecognizer(target: self, action: #selector(held(_:)))
    hold.cancelsTouchesInView = false
    hold.minimumPressDuration = 0.25
    addGestureRecognizer(hold)
  }
  required init?(coder: NSCoder) { nil }
  @objc private func held(_ gesture: UILongPressGestureRecognizer) {
    switch gesture.state {
    case .began:
      UIImpactFeedbackGenerator(style: .light).impactOccurred()
      UIView.animate(withDuration: 0.15) { self.thumb.transform = CGAffineTransform(scaleX: 2, y: 1) }
    case .ended, .cancelled, .failed:
      UIView.animate(withDuration: 0.15) { self.thumb.transform = .identity }
    default: break
    }
  }
  private var extent: CGFloat {
    guard let table else { return 0 }
    return max(
      0,
      table.contentSize.height + table.adjustedContentInset.top + table.adjustedContentInset.bottom
        - table.bounds.height)
  }
  override func layoutSubviews() {
    super.layoutSubviews()
    guard let table, bounds.height > 0, extent > 0 else { return }
    let fraction = min(1, max(0, (table.contentOffset.y + table.adjustedContentInset.top) / extent))
    let length = min(
      bounds.height, max(36, bounds.height * table.bounds.height / (extent + table.bounds.height)))
    thumb.bounds.size = CGSize(width: 6, height: length)
    thumb.center = CGPoint(x: bounds.midX, y: fraction * (bounds.height - length) + length / 2)
    thumb.backgroundColor = table.tintColor
    thumb.layer.cornerRadius = 3
    accessibilityValue = "\(Int((fraction * 100).rounded()))%"
  }
  override func beginTracking(_ touch: UITouch, with event: UIEvent?) -> Bool {
    let y = touch.location(in: self).y
    grabOffset = thumb.frame.minY...thumb.frame.maxY ~= y ? y - thumb.frame.minY : thumb.bounds.midY
    scroll(to: y)
    return true
  }
  override func continueTracking(_ touch: UITouch, with event: UIEvent?) -> Bool {
    scroll(to: touch.location(in: self).y)
    return true
  }
  private func scroll(to y: CGFloat) {
    guard let table else { return }
    let fraction = min(1, max(0, (y - grabOffset) / max(1, bounds.height - thumb.bounds.height)))
    table.setContentOffset(
      CGPoint(x: table.contentOffset.x, y: fraction * extent - table.adjustedContentInset.top),
      animated: false)
  }
  private func page(_ direction: CGFloat) {
    guard let table else { return }
    let top = -table.adjustedContentInset.top
    table.setContentOffset(
      CGPoint(
        x: table.contentOffset.x,
        y: min(top + extent, max(top, table.contentOffset.y + direction * table.bounds.height))),
      animated: false)
  }
  override func accessibilityIncrement() { page(1) }
  override func accessibilityDecrement() { page(-1) }
}
