import SwiftUI
import UIKit

private final class PlaylistTable: UITableView {
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
  let showsSelection: Bool
  let keyboardTarget: Bool
  let selecting: Bool
  let beginSelecting: () -> Void
  var dismissFilterKeyboard: () -> Void

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
    private var rowHeight: CGFloat = 76
    private var artworkRowHeight: CGFloat = 330
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
      let structureChanged =
        reset || old.count != next.column.count || old.sections != next.column.sections
        || old.largeArtwork != next.column.largeArtwork
        || old.artworkStart != next.column.artworkStart || height != rowHeight
      let editing = next.selecting || next.column.playlistEditing == true
      let contentChanged =
        parent.selecting != next.selecting || old.playlistEditing != next.column.playlistEditing || old.rows != next.column.rows
        || parent.showsSelection != next.showsSelection
        || theme != next.model.theme
        || parent.colorScheme != next.colorScheme
      theme = next.model.theme
      rowHeight = height
      artworkRowHeight = 268 + UIFontMetrics(forTextStyle: .headline).scaledValue(for: 62)
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
      table.backgroundColor = UIColor(next.model.theme.bg)
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
          // Detach hosted controls before replacing row identities.
          for cell in table.visibleCells {
            cell.contentConfiguration = nil
            cell.accessibilityIdentifier = nil
          }
          table.reloadData()
          table.layoutIfNeeded()
          if reset, next.column.count > 0 {
            let target = old.filter != next.column.filter ? 0 : next.column.selectedPosition ?? 0
            table.scrollToRow(
              at: indexPath(min(target, next.column.count - 1)), at: .top, animated: false)
          }
        } else {
          for path in table.indexPathsForVisibleRows ?? [] {
            if let cell = table.cellForRow(at: path) { configure(cell, at: path) }
          }
        }
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

    func numberOfSections(in tableView: UITableView) -> Int { sections.count }
    func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
      let end = section + 1 < sections.count ? sections[section + 1].position : parent.column.count
      return max(0, end - sections[section].position)
    }
    func tableView(_ tableView: UITableView, cellForRowAt path: IndexPath) -> UITableViewCell {
      let cell = tableView.dequeueReusableCell(withIdentifier: "music", for: path)
      configure(cell, at: path)
      return cell
    }
    private func position(_ path: IndexPath) -> Int { sections[path.section].position + path.row }
    private func indexPath(_ position: Int) -> IndexPath {
      let section = sections.lastIndex(where: { $0.position <= position }) ?? 0
      return IndexPath(row: position - sections[section].position, section: section)
    }
    private func configure(_ cell: UITableViewCell, at path: IndexPath) {
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
            large: parent.column.largeArtwork && row.kind == "album" && row.artwork != nil,
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
      let large =
        parent.column.largeArtwork
        && parent.column.artworkStart.map { position(path) >= $0 } == true
      return large ? artworkRowHeight : rowHeight
    }
    func tableView(_ tableView: UITableView, didSelectRowAt path: IndexPath) {
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
      _ tableView: UITableView, contextMenuConfigurationForRowAt path: IndexPath, point: CGPoint
    ) -> UIContextMenuConfiguration? {
      guard let commands = rows[position(path)]?.context, !commands.isEmpty else { return nil }
      let model = parent.model
      return UIContextMenuConfiguration(identifier: nil, previewProvider: nil) { _ in
        UIMenu(
          children: commands.menuSections.map { section in
            UIMenu(
              options: .displayInline,
              children: section.map { command in
                UIAction(title: command.label, attributes: command.disabled == nil ? [] : .disabled)
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
      let positions = visible.map(position)
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
    let railWidth: CGFloat = show ? 28 : 0
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
