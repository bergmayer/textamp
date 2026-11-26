import UIKit
import XCTest

final class LibraryFlowTests: XCTestCase {
  override func setUp() {
    super.setUp()
    addUIInterruptionMonitor(withDescription: "App icon confirmation") { alert in
      guard alert.staticTexts.matching(NSPredicate(format: "label CONTAINS[c] 'icon'")).count > 0,
        alert.buttons["OK"].exists
      else { return false }
      alert.buttons["OK"].tap()
      return true
    }
  }

  func testTabletLibraryPanes() throws {
    try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .pad)
    continueAfterFailure = false
    let original = XCUIDevice.shared.orientation
    XCUIDevice.shared.orientation = .landscapeLeft
    defer { XCUIDevice.shared.orientation = original }
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    if app.buttons["Add Library"].waitForExistence(timeout: 2) { connectFixture(app) }

    let queue = app.navigationBars["Play Queue"]
    let toggleQueue = app.buttons["bottom-queue"]
    if !queue.exists { toggleQueue.tap() }
    let artists = app.buttons["Artists"]
    artists.tap()
    XCTAssertTrue(app.buttons["Queue Type"].isHittable)
    let artist = app.staticTexts["A Artist"].firstMatch
    XCTAssertTrue(artist.waitForExistence(timeout: 8))
    artist.tap()
    let album = albumEntry(app, "A Album 1 (2000)")
    XCTAssertTrue(album.waitForExistence(timeout: 8))
    XCTAssertTrue(artists.isHittable)
    XCTAssertLessThan(artists.frame.maxX, artist.frame.minX)
    XCTAssertLessThan(artist.frame.maxX, album.frame.minX)
    album.tap()
    let track = app.tables.staticTexts["A Track 1.03"].firstMatch
    XCTAssertTrue(track.waitForExistence(timeout: 8))
    track.tap()
    let details = app.otherElements["workspace-column-header-Details"]
    XCTAssertTrue(details.waitForExistence(timeout: 8))
    XCTAssertTrue(track.isHittable)
    XCTAssertLessThan(track.frame.maxX, details.frame.minX)
    let sibling = app.tables.staticTexts["A Track 1.02"].firstMatch
    sibling.tap()
    XCTAssertTrue(
      app.scrollViews.staticTexts["A Track 1.02"].waitForExistence(timeout: 5))
    XCTAssertTrue(artists.isHittable)
    XCTAssertLessThanOrEqual(details.frame.maxX, queue.frame.minX + 1)
    let sidebarButton = app.buttons.matching(
      NSPredicate(
        format: "label CONTAINS[c] 'sidebar' OR identifier == 'displayModeButtonItem'"
      )
    ).firstMatch
    sidebarButton.tap()
    XCTAssertFalse(artists.exists && artists.isHittable)
    XCTAssertTrue(details.buttons["Close Column"].isHittable)
    XCTAssertTrue(queue.isHittable)
    sidebarButton.tap()
    XCTAssertTrue(artists.isHittable)
    XCTAssertTrue(app.scrollViews["detail-content"].staticTexts["A Track 1.02"].exists)
    XCTAssertTrue(details.buttons["Close Column"].isHittable)
    toggleQueue.tap()
    XCTAssertFalse(queue.exists)
    XCTAssertTrue(details.exists)
    runCommand(app, query: "Play Queue", label: "Play Queue")
    XCTAssertTrue(queue.waitForExistence(timeout: 5))
    XCTAssertTrue(details.buttons["Close Column"].isHittable)
    capture(app, "iPad Library and Track Details")
    XCTAssertTrue(artists.isSelected)
    app.scrollViews["detail-content"].buttons["Add to Queue"].tap()
    XCTAssertTrue(app.buttons["Play A Track 1.02"].waitForExistence(timeout: 5))
    queue.buttons["Queue Tools"].tap()
    app.buttons["Edit Queue"].tap()
    let finishEditing = queue.buttons["Done"]
    XCTAssertTrue(finishEditing.waitForExistence(timeout: 5))
    finishEditing.tap()
    app.buttons["Play A Track 1.02"].tap()
    openNowPlaying(app)
    let closePlayer = app.buttons["Close Now Playing"]
    XCTAssertTrue(closePlayer.waitForExistence(timeout: 5))
    XCTAssertTrue(app.buttons["Queue Type"].isHittable)
    XCTAssertTrue(app.buttons["now-playing-toggle"].isHittable)
    capture(app, "iPad Now Playing")
    closePlayer.tap()
    XCTAssertTrue(details.waitForExistence(timeout: 5))
    XCUIDevice.shared.orientation = .portrait
    XCTAssertTrue(details.waitForExistence(timeout: 5))
    capture(app, "iPad Portrait Details")
    details.buttons["Close Column"].tap()
    XCTAssertTrue(track.waitForExistence(timeout: 5))
    XCTAssertFalse(details.exists)
    XCTAssertTrue(artists.isHittable)
    XCTAssertTrue(app.buttons["Queue Tools"].waitForExistence(timeout: 5))
    sidebarButton.tap()
    XCTAssertFalse(app.buttons["bottom-library"].exists)
    sidebarButton.tap()
    XCTAssertTrue(artists.isHittable)
    XCTAssertTrue(track.isHittable)
  }

  func testTabletCanUndoAlbumQueueReplacement() throws {
    try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .pad)
    continueAfterFailure = false
    let original = XCUIDevice.shared.orientation
    XCUIDevice.shared.orientation = .portrait
    defer { XCUIDevice.shared.orientation = original }
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    if app.buttons["Add Library"].waitForExistence(timeout: 2) { connectFixture(app) }

    let queue = app.navigationBars["Play Queue"]
    if !queue.exists { app.buttons["bottom-queue"].tap() }
    app.buttons["Artists"].tap()
    let artist = app.staticTexts["A Artist"].firstMatch
    XCTAssertTrue(artist.waitForExistence(timeout: 8))
    artist.tap()
    func playAlbum(_ number: Int) {
      let album = app.staticTexts["A Album \(number) (2000)"].firstMatch
      XCTAssertTrue(album.waitForExistence(timeout: 8))
      album.tap()
      let play = app.buttons["Play Album"].firstMatch
      XCTAssertTrue(play.waitForExistence(timeout: 5))
      play.tap()
    }

    playAlbum(1)
    let previous = app.buttons["Play A Track 1.01"]
    let replacement = app.buttons["Play A Track 2.01"]
    XCTAssertTrue(previous.waitForExistence(timeout: 8))
    let close = app.buttons.matching(identifier: "Close Column")
      .allElementsBoundByIndex.last(where: \.isHittable)
    try XCTUnwrap(close).tap()
    playAlbum(2)
    XCTAssertTrue(replacement.waitForExistence(timeout: 8))
    XCTAssertFalse(previous.exists)
    queue.buttons["Queue Tools"].tap()
    app.buttons["Undo queue edit"].tap()
    XCTAssertTrue(previous.waitForExistence(timeout: 5))
    XCTAssertFalse(replacement.exists)
  }

  func testQueueHeaderMenus() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    XCTAssertTrue(app.buttons["bottom-queue"].waitForExistence(timeout: 5))
    if !app.navigationBars["Play Queue"].exists { app.buttons["bottom-queue"].tap() }
    dismissIconNotice(XCUIApplication(bundleIdentifier: "com.apple.springboard"), timeout: 2)
    let type = app.buttons["Queue Type"]
    let tools = app.buttons["Queue Tools"]
    XCTAssertTrue(type.waitForExistence(timeout: 5))
    XCTAssertTrue(tools.isHittable)
    XCTAssertLessThan(abs(type.frame.midY - tools.frame.midY), 2)
    capture(app, "Queue Header Icons")
    type.tap()
    XCTAssertTrue(app.buttons["Manual Queue"].waitForExistence(timeout: 5))
    let repeatQueue = app.buttons["Repeat Queue"]
    let related = app.buttons["Continue with Related Tracks"]
    XCTAssertTrue(repeatQueue.exists)
    XCTAssertTrue(related.exists)
    for _ in 0..<3 where !related.isHittable {
      app.coordinate(withNormalizedOffset: CGVector(dx: 0.65, dy: 0.55))
        .press(
          forDuration: 0.1,
          thenDragTo: app.coordinate(
            withNormalizedOffset: CGVector(dx: 0.65, dy: 0.25)))
    }
    XCTAssertTrue(related.isHittable)
    XCTAssertGreaterThan(related.frame.minY, repeatQueue.frame.minY)
    capture(app, "Queue Type Menu Playback")
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.1, dy: 0.6)).tap()
    tools.tap()
    let save = app.buttons["Save queue as playlist"]
    let undo = app.buttons["Undo queue edit"]
    let clear = app.buttons["Clear Queue"]
    XCTAssertTrue(save.waitForExistence(timeout: 5))
    XCTAssertTrue(undo.exists)
    XCTAssertTrue(clear.exists)
    XCTAssertLessThan(save.frame.minY, undo.frame.minY)
    XCTAssertLessThan(undo.frame.minY, clear.frame.minY)
    for label in ["Shuffle", "Twofer", "Auto-Mix"] {
      let tool = app.buttons[label]
      XCTAssertTrue(tool.exists)
      XCTAssertFalse(tool.isEnabled)
    }
    if app.buttons["Edit Queue"].exists { XCTAssertFalse(app.buttons["Edit Queue"].isEnabled) }
    XCTAssertFalse(undo.isEnabled)
    XCTAssertFalse(save.isEnabled)
    XCTAssertFalse(clear.isEnabled)
    capture(app, "Queue Tools Menu")
  }

  func testDownloadItemContextMenus() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    connectFixture(app)
    for cancelPending in [true, false] {
      if !app.buttons["Artists"].exists { app.buttons["bottom-library"].tap() }
      app.buttons["Artists"].tap()
      XCTAssertTrue(app.staticTexts["A Artist"].waitForExistence(timeout: 10))
      app.staticTexts["A Artist"].tap()
      XCTAssertTrue(albumEntry(app, "A Album 1 (2000)").waitForExistence(timeout: 10))
      albumEntry(app, "A Album 1 (2000)").tap()
      let track = app.staticTexts["A Track 1.03"].firstMatch
      XCTAssertTrue(track.waitForExistence(timeout: 10))
      track.tap()
      XCTAssertTrue(app.buttons["Download"].waitForExistence(timeout: 5))
      app.buttons["Download"].tap()
      let notice = app.buttons["Downloading music…"].firstMatch
      XCTAssertTrue(notice.waitForExistence(timeout: 5))
      notice.tap()
      let pending = app.buttons["Cancel Download A Track 1.03"].firstMatch
      XCTAssertTrue(pending.waitForExistence(timeout: 5))
      if !cancelPending { XCTAssertTrue(pending.waitForNonExistence(timeout: 35)) }
      let row = app.staticTexts["A Track 1.03"].firstMatch
      XCTAssertTrue(row.waitForExistence(timeout: 5))
      row.press(forDuration: 0.7)
      let label = cancelPending ? "Cancel Download" : "Remove Download"
      let action = app.buttons[label].firstMatch
      XCTAssertTrue(action.waitForExistence(timeout: 5), app.debugDescription)
      capture(app, label + " Context Menu")
      action.tap()
      XCTAssertTrue(app.staticTexts["No items"].waitForExistence(timeout: 5))
    }
  }

  func testPendingDownloadsNotificationAndCancellation() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    connectFixture(app)
    app.buttons["Artists"].tap()
    XCTAssertTrue(app.staticTexts["A Artist"].waitForExistence(timeout: 10))
    app.staticTexts["A Artist"].tap()
    XCTAssertTrue(albumEntry(app, "A Album 1 (2000)").waitForExistence(timeout: 10))
    albumEntry(app, "A Album 1 (2000)").tap()
    let track = app.staticTexts["A Track 1.03"].firstMatch
    XCTAssertTrue(track.waitForExistence(timeout: 10))
    track.tap()
    XCTAssertTrue(app.buttons["Download"].waitForExistence(timeout: 5))
    app.buttons["Download"].tap()
    let notice = app.buttons["Downloading music…"].firstMatch
    XCTAssertTrue(notice.waitForExistence(timeout: 5))
    notice.tap()
    let cancel = app.buttons["Cancel Download A Track 1.03"].firstMatch
    XCTAssertTrue(cancel.waitForExistence(timeout: 5), app.debugDescription)
    XCTAssertTrue(app.staticTexts["download-summary"].exists)
    capture(app, "Pending Download")
    cancel.tap()
    let removed = cancel.waitForNonExistence(timeout: 5)
    XCTAssertTrue(removed, app.debugDescription)
    XCTAssertTrue(app.staticTexts["No items"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.buttons["column-title"].isHittable)
    XCTAssertTrue(app.buttons["bottom-library"].isHittable)
    capture(app, "Cancelled Download")
  }

  func testDownloadedMusicSummaryAndCachedTrackSearch() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    app.buttons["bottom-search"].tap()
    let search = app.textFields["Search your library"]
    XCTAssertTrue(search.waitForExistence(timeout: 5))
    search.tap()
    search.typeText("A Track 1.03")
    let result = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'A Track 1.03'"))
      .firstMatch
    XCTAssertTrue(result.waitForExistence(timeout: 8))
    result.tap()
    XCTAssertTrue(app.staticTexts["Details"].waitForExistence(timeout: 5))
    if !app.buttons["Remove Download"].exists {
      app.buttons["Download"].tap()
      XCTAssertTrue(app.buttons["Remove Download"].waitForExistence(timeout: 15))
    }
    runCommand(app, query: "Downloads", label: "Downloads")
    let summary = app.staticTexts["download-summary"]
    capture(app, "Downloads Destination")
    XCTAssertTrue(summary.waitForExistence(timeout: 5), app.debugDescription)
    XCTAssertTrue(summary.label.contains(" item"))
    XCTAssertTrue(summary.label.contains(" · "))
    XCTAssertFalse(summary.label.hasSuffix("· 0 B"))
    XCTAssertLessThan(summary.frame.maxY, app.buttons["bottom-library"].frame.minY)
    XCTAssertTrue(app.staticTexts["A Track 1.03"].exists)
    capture(app, "Downloaded Music Storage Summary")

    runCommand(app, query: "Toggle Offline Mode", label: "Toggle Offline Mode")
    let offline = app.descendants(matching: .any)["offline-status"].firstMatch
    XCTAssertTrue(offline.waitForExistence(timeout: 5))
    app.buttons["View Options"].tap()
    app.buttons["Group by album"].tap()
    let album = albumEntry(app, "A Album 1 (2000)")
    XCTAssertTrue(album.waitForExistence(timeout: 5), app.debugDescription)
    album.tap()
    XCTAssertTrue(app.buttons["A Artist"].firstMatch.waitForExistence(timeout: 5))
    app.buttons["A Artist"].firstMatch.tap()
    let albums = app.staticTexts.matching(
      NSPredicate(format: "label CONTAINS %@", "Downloaded albums")
    ).firstMatch
    XCTAssertTrue(albums.waitForExistence(timeout: 5), app.debugDescription)
  }

  func testSearchJourneyAndDownloadControls() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    connectFixture(app)
    let commands = app.buttons["Command Palette"].firstMatch
    XCTAssertGreaterThan(commands.frame.minY, app.frame.height * 0.8)
    XCTAssertFalse(app.navigationBars.buttons["Command Palette"].exists)
    XCTAssertFalse(app.buttons["More"].exists)
    app.buttons["bottom-search"].tap()
    let search = app.textFields["Search your library"]
    XCTAssertTrue(search.waitForExistence(timeout: 5))
    XCTAssertFalse(app.navigationBars["Search"].buttons["Command Palette"].exists)
    app.buttons["Done"].tap()
    XCTAssertTrue(app.buttons["bottom-library"].isSelected)
    runCommand(app, query: "Journey", label: "Journey…")
    let start = app.buttons["Start track"]
    XCTAssertTrue(start.waitForExistence(timeout: 5))
    start.tap()
    let field = app.textFields["Find a track"]
    XCTAssertTrue(field.waitForExistence(timeout: 5))
    field.tap()
    field.typeText("A Track 1.03")
    let choice = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'A Track 1.03'"))
      .firstMatch
    XCTAssertTrue(choice.waitForExistence(timeout: 8))
    capture(app, "Journey Track Picker")
    choice.tap()
    XCTAssertTrue(start.waitForExistence(timeout: 5))
    XCTAssertTrue((start.value as? String)?.contains("A Track 1.03") == true)
    start.tap()
    XCTAssertTrue(field.waitForExistence(timeout: 5))
    field.tap()
    field.typeText("other")
    app.navigationBars["Start track"].buttons["Done"].tap()
    XCTAssertTrue((start.value as? String)?.contains("A Track 1.03") == true)
    app.buttons["Done"].tap()

    app.buttons["bottom-search"].tap()
    XCTAssertTrue(search.waitForExistence(timeout: 5))
    search.tap()
    search.typeText("A Track 1.03")
    XCTAssertTrue(choice.waitForExistence(timeout: 8))
    choice.tap()
    XCTAssertTrue(app.staticTexts["Details"].waitForExistence(timeout: 5))
    let download = app.buttons["Download"]
    if app.buttons["Remove Download"].exists { app.buttons["Remove Download"].tap() }
    XCTAssertTrue(download.waitForExistence(timeout: 5))
    XCTAssertFalse(download.staticTexts.firstMatch.exists)
    XCTAssertEqual(download.frame.height, app.buttons["Play Track"].frame.height, accuracy: 1)
    download.tap()
    XCTAssertTrue(app.buttons["Remove Download"].waitForExistence(timeout: 15))
    capture(app, "Track Download Control")
    app.buttons["bottom-search"].tap()
    XCTAssertTrue(search.waitForExistence(timeout: 5))
    search.tap()
    search.typeText("A Album 1")
    let album = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'A Album 1'"))
      .firstMatch
    XCTAssertTrue(album.waitForExistence(timeout: 8))
    album.tap()
    XCTAssertTrue(app.staticTexts["Details"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.buttons["Download Album"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["Download Album"].staticTexts.firstMatch.exists)
    XCTAssertEqual(
      app.buttons["Download Album"].frame.height, app.buttons["Play Album"].frame.height,
      accuracy: 1)
    capture(app, "Album Download Control")
    app.buttons["bottom-library"].tap()
    for _ in 0..<6 where !app.buttons["sidebar-open-Playlists"].exists {
      app.navigationBars.buttons.element(boundBy: 0).tap()
    }
    XCTAssertTrue(app.buttons["sidebar-open-Playlists"].exists)
    app.buttons["sidebar-open-Playlists"].tap()
    let playlist = app.staticTexts["Fixture Playlist"]
    XCTAssertTrue(playlist.waitForExistence(timeout: 8))
    playlist.press(forDuration: 0.8)
    XCTAssertTrue(app.buttons["Download"].waitForExistence(timeout: 5))
    capture(app, "Playlist Download Menu")
    app.buttons["Download"].tap()
    runCommand(app, query: "Manage Downloads", label: "Manage Downloads")
    XCTAssertTrue(
      app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH '30 tracks'"))
        .firstMatch.waitForExistence(timeout: 20))
  }

  func testBackMenuSeparatesParentAndPreviousLocation() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    connectFixture(app)
    app.buttons["bottom-search"].tap()
    let search = app.textFields["Search your library"]
    XCTAssertTrue(search.waitForExistence(timeout: 5))
    search.tap()
    search.typeText("A Track 1.03")
    let result = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'A Track 1.03'"))
      .firstMatch
    XCTAssertTrue(result.waitForExistence(timeout: 8))
    result.tap()
    XCTAssertTrue(app.staticTexts["Details"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["Return to Previous Location"].exists)
    app.buttons["Back"].tap()
    XCTAssertFalse(app.staticTexts["Details"].exists)
    app.buttons["Back"].press(forDuration: 0.8)
    XCTAssertTrue(app.buttons["Up One Level"].waitForExistence(timeout: 3))
    XCTAssertTrue(app.buttons["Return to Previous Location"].exists)
    capture(app, "Back Navigation Menu")
    app.buttons["Return to Previous Location"].tap()
    XCTAssertTrue(app.staticTexts["library-heading"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["Back"].exists)
  }

  func testTrackDetailsPlaybackMenuAndHierarchyBack() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    connectFixture(app)
    app.buttons["Command Palette"].firstMatch.tap()
    XCTAssertTrue(app.buttons["Play Random Albums"].waitForExistence(timeout: 5))
    app.navigationBars["Command Palette"].buttons["Done"].tap()
    app.buttons["bottom-search"].tap()
    let search = app.textFields["Search your library"]
    XCTAssertTrue(search.waitForExistence(timeout: 5))
    search.tap()
    search.typeText("A Track 1.03")
    let result = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'A Track 1.03'"))
      .firstMatch
    XCTAssertTrue(result.waitForExistence(timeout: 8))
    result.tap()
    XCTAssertTrue(app.staticTexts["Details"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.staticTexts["A Track 1.03"].exists)
    let play = app.buttons["Play Track"]
    XCTAssertTrue(play.exists)
    capture(app, "Track Details Playback")
    play.press(forDuration: 0.8)
    XCTAssertTrue(app.buttons["Play Next"].waitForExistence(timeout: 5))
    capture(app, "Track Playback Menu")
    app.buttons["Play Next"].tap()
    let toast = app.buttons["Dismiss Status"]
    XCTAssertTrue(toast.waitForExistence(timeout: 3))
    let expired = expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: toast)
    wait(for: [expired], timeout: 10)
    app.buttons["Back"].tap()
    XCTAssertFalse(app.staticTexts["Details"].exists)
    XCTAssertFalse(app.buttons["Return to Previous Location"].exists)
    app.buttons["Back"].press(forDuration: 0.8)
    XCTAssertTrue(app.buttons["Up One Level"].waitForExistence(timeout: 3))
    XCTAssertTrue(app.buttons["Return to Previous Location"].exists)
    app.buttons["Return to Previous Location"].tap()
    app.buttons["bottom-library"].tap()
    XCTAssertTrue(app.staticTexts["library-heading"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["Return to Previous Location"].exists)
    capture(app, "Library Root Title")
  }

  func testBottomControlsRemainUsableAcrossPresentations() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    capture(app, "Library Bottom Controls")
    XCTAssertGreaterThan(
      app.buttons["bottom-queue"].frame.midX,
      app.buttons["Command Palette"].firstMatch.frame.midX)
    app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Now Playing, '")).firstMatch
      .swipeUp()

    XCTAssertTrue(app.navigationBars["Now Playing"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.navigationBars["Now Playing"].buttons["Command Palette"].exists)
    checkPalette(app)
    app.buttons["Close Now Playing"].tap()
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Textamp"].tap()
    app.buttons["Settings…"].tap()
    XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.navigationBars["Settings"].buttons["Command Palette"].exists)
    XCTAssertGreaterThan(
      app.buttons.matching(identifier: "Command Palette").allElementsBoundByIndex.first(where: {
        $0.isHittable
      })!.frame
        .minY, app.frame.height * 0.8)
    capture(app, "Settings Bottom Commands")
    checkPalette(app)
    app.navigationBars["Settings"].buttons["Done"].tap()
    app.buttons["bottom-search"].tap()
    XCTAssertTrue(app.textFields["Search your library"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.navigationBars["Search"].buttons["Command Palette"].exists)
    app.buttons["Done"].tap()
    XCTAssertTrue(app.buttons["bottom-library"].isSelected)
  }
  private func sweep(_ start: CGRect, _ end: CGRect, horizontalFraction: CGFloat = 0.5) {
    _ = perform(
      NSSelectorFromString("selectPlaylistRowsFrom:to:"),
      with: NSValue(cgPoint: CGPoint(x: start.minX + horizontalFraction * start.width, y: start.midY)),
      with: NSValue(cgPoint: CGPoint(x: end.minX + horizontalFraction * end.width, y: end.midY)))
  }

  func testTwoFingerSwipeSelectsAlbumTracksWhileBrowsing() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    app.buttons["sidebar-open-Albums"].tap()
    app.staticTexts["All"].tap()
    let album = albumEntry(app, "A Album 1 (2000)")
    XCTAssertTrue(album.waitForExistence(timeout: 8))
    album.tap()
    let table = app.tables["library-column"]
    let first = table.cells.element(boundBy: 0)
    let second = table.cells.element(boundBy: 1)
    XCTAssertTrue(second.waitForExistence(timeout: 8))
    sweep(first.frame, second.frame, horizontalFraction: 0.15)
    XCTAssertTrue(app.buttons["selection-done"].waitForExistence(timeout: 5))
    let third = table.cells.element(boundBy: 2)
    let fourth = table.cells.element(boundBy: 3)
    sweep(fourth.frame, third.frame, horizontalFraction: 0.82)
    sweep(first.frame, second.frame)
    XCTAssertFalse(first.isSelected)
    XCTAssertFalse(second.isSelected)
    third.press(forDuration: 0.7)
    XCTAssertTrue(app.buttons["Add Selected Tracks to Playlist…"].firstMatch.waitForExistence(timeout: 5))
  }

  func testPlaylistEditChecksAndMovesTracks() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    app.buttons["sidebar-open-Playlists"].tap()
    app.tables["library-column"].staticTexts["Fixture Playlist"].tap()
    let table = app.tables["library-column"]
    let edit = app.buttons["playlist-edit"].firstMatch
    XCTAssertTrue(edit.waitForExistence(timeout: 8))
    let heading = app.buttons["column-title"].firstMatch
    if UIDevice.current.userInterfaceIdiom == .phone {
      XCTAssertEqual(app.navigationBars.buttons["Back"].frame.midY, edit.frame.midY, accuracy: 2)
      XCTAssertLessThan(edit.frame.midY, heading.frame.midY)
    } else if !app.navigationBars.buttons["Back"].exists {
      XCTAssertEqual(heading.frame.midY, edit.frame.midY, accuracy: 2)
    }
    edit.tap()
    XCTAssertEqual(edit.label, "Done")
    let first = table.cells.element(boundBy: 0)
    let second = table.cells.element(boundBy: 1)
    let trash = app.buttons["playlist-remove"]
    XCTAssertTrue(trash.exists)
    XCTAssertFalse(trash.isEnabled)
    let firstKey = first.identifier
    let secondKey = second.identifier
    sweep(first.frame, second.frame)
    app.buttons["Selected Tracks"].firstMatch.tap()
    app.buttons["Move Down in Playlist"].tap()
    let moved = expectation(
      for: NSPredicate { _, _ in
        table.cells.element(boundBy: 1).identifier == firstKey
          && table.cells.element(boundBy: 2).identifier == secondKey
      }, evaluatedWith: table)
    wait(for: [moved], timeout: 8)
    edit.tap()
    XCTAssertEqual(edit.label, "Edit")
    let start = second.frame
    let end = table.cells.element(boundBy: 2).frame
    sweep(start, end)
    let gesture = expectation(for: NSPredicate(format: "label == 'Done'"), evaluatedWith: edit)
    wait(for: [gesture], timeout: 5)
    app.buttons["Selected Tracks"].firstMatch.tap()
    app.buttons["Move Up in Playlist"].tap()
    let restored = expectation(
      for: NSPredicate { _, _ in
        first.identifier == firstKey && second.identifier == secondKey
      }, evaluatedWith: table)
    wait(for: [restored], timeout: 8)
    let nextKey = table.cells.element(boundBy: 2).identifier
    XCTAssertTrue(trash.isEnabled)
    trash.tap()
    let removed = expectation(for: NSPredicate { _, _ in
      first.identifier == nextKey && !table.cells[firstKey].exists && !table.cells[secondKey].exists
    }, evaluatedWith: table)
    wait(for: [removed], timeout: 8)
  }
  func testLibraryHeadingStaysFixedWhileRowsScroll() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    let heading = app.staticTexts["library-heading"]
    XCTAssertTrue(heading.waitForExistence(timeout: 5))
    XCTAssertTrue(app.buttons["Switch Library"].isHittable)
    XCTAssertTrue(app.buttons["Settings"].firstMatch.isHittable)
    let originalFrame = heading.frame
    app.buttons["sidebar-disclosure-Albums"].tap()
    let row = app.buttons["Random"]
    XCTAssertTrue(row.waitForExistence(timeout: 5))
    let before = row.frame.minY
    app.swipeUp()
    XCTAssertTrue(!row.isHittable || row.frame.minY < before - 10)
    XCTAssertEqual(heading.frame, originalFrame)
    XCTAssertFalse(app.navigationBars.staticTexts["Library"].exists)
    app.swipeDown()
    app.buttons["sidebar-open-Albums"].tap()
    let albums = app.buttons["column-title"]
    XCTAssertTrue(albums.waitForExistence(timeout: 5))
    XCTAssertFalse(heading.exists)
    XCTAssertTrue(app.buttons["Back"].isHittable)
  }

  func testRadioDisclosureAndColumn() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=radio-sidebar"]
    app.launch()
    let albums = app.buttons["sidebar-open-Albums"]
    if !albums.waitForExistence(timeout: 5) { connectFixture(app) }
    let playlists = app.buttons["sidebar-open-Playlists"]
    XCTAssertTrue(playlists.waitForExistence(timeout: 8))
    XCTAssertEqual(albums.frame.maxY, playlists.frame.minY, accuracy: 12)
    let arrow = app.buttons["sidebar-disclosure-Radio"]
    for _ in 0..<4 where !arrow.isHittable { app.swipeUp() }
    XCTAssertTrue(arrow.waitForExistence(timeout: 5))
    arrow.tap()
    XCTAssertTrue(app.buttons["Play Random Albums"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.staticTexts["library-heading"].exists)
    if !arrow.isHittable { app.swipeDown() }
    arrow.tap()
    XCTAssertFalse(app.buttons["Play Random Albums"].exists)
    app.buttons["sidebar-open-Radio"].tap()
    XCTAssertTrue(app.buttons["About Play Random Albums"].waitForExistence(timeout: 5))
  }

  func testPlaylistsDisclosure() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-playlists"]
    app.launch()
    if !app.buttons["sidebar-open-Playlists"].waitForExistence(timeout: 5) { connectFixture(app) }
    let arrow = app.buttons["sidebar-disclosure-Playlists"]
    let playlist = app.buttons["Fixture Playlist"]
    XCTAssertTrue(arrow.waitForExistence(timeout: 8))
    arrow.tap()
    XCTAssertTrue(playlist.waitForExistence(timeout: 5))
    XCTAssertTrue(app.staticTexts["library-heading"].exists)
    arrow.tap()
    XCTAssertFalse(playlist.exists)
    app.buttons["sidebar-open-Playlists"].tap()
    XCTAssertTrue(
      app.tables["library-column"].staticTexts["Fixture Playlist"].waitForExistence(timeout: 5))
  }
  func testSidebarDisclosureAndColumn() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    connectFixture(app)
    let arrow = app.buttons["sidebar-disclosure-Albums"]
    XCTAssertTrue(arrow.waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["Random"].exists)
    arrow.tap()
    XCTAssertTrue(app.buttons["Random"].waitForExistence(timeout: 3))
    XCTAssertTrue(app.staticTexts["library-heading"].exists)
    arrow.tap()
    XCTAssertFalse(app.buttons["Random"].exists)
    app.buttons["sidebar-open-Albums"].tap()
    XCTAssertTrue(app.tables["library-column"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.staticTexts["Random"].exists)
    capture(app, "Album Category Column")
    let categories = app.tables["library-column"]
    let compilationsRow = categories.staticTexts["Compilations"]
    if !compilationsRow.isHittable || compilationsRow.frame.maxY > categories.frame.maxY {
      categories.swipeUp()
    }
    compilationsRow.tap()
    let columnTitle = app.buttons["column-title"]
    let compilations = expectation(
      for: NSPredicate(format: "label == 'compilations'"), evaluatedWith: columnTitle)
    wait(for: [compilations], timeout: 5)
    app.navigationBars.buttons.element(boundBy: 0).tap()
    let albums = expectation(
      for: NSPredicate(format: "label == 'Albums'"), evaluatedWith: columnTitle)
    wait(for: [albums], timeout: 5)
    XCTAssertTrue(app.staticTexts["Random"].exists)
    app.staticTexts["All"].tap()
    XCTAssertTrue(albumEntry(app, "A Album 1 (2000)").waitForExistence(timeout: 8))
    app.navigationBars.buttons.element(boundBy: 0).tap()
    XCTAssertTrue(app.staticTexts["Random"].waitForExistence(timeout: 5))
    app.navigationBars.buttons.element(boundBy: 0).tap()
    XCTAssertTrue(app.buttons["sidebar-open-Albums"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["Random"].exists)
  }

  func testPaletteReplacesMoreAndIncludesSettingsInTextamp() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    XCTAssertFalse(app.buttons["More"].exists)
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Textamp"].tap()
    app.buttons["Settings…"].tap()
    XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
  }

  func testLibraryControlsAndContextualMusicSearch() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    let artists = app.buttons["Artists"]
    let albums = app.buttons["sidebar-open-Albums"]
    capture(app, "Compact Library")
    let artistLabel = artists.staticTexts["Artists"]
    let albumLabel = albums.staticTexts["Albums"]
    XCTAssertEqual(artistLabel.frame.minX, albumLabel.frame.minX, accuracy: 1)
    XCTAssertEqual(artistLabel.frame.height, albumLabel.frame.height, accuracy: 1)
    XCTAssertLessThanOrEqual(albums.frame.height, 54)
    app.buttons["Command Palette"].firstMatch.tap()
    let commandQuery = app.textFields["Find a command"]
    commandQuery.tap()
    commandQuery.typeText("Search")
    XCTAssertTrue(app.buttons["Search Apple Music"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.buttons["Search YouTube"].exists)
    XCTAssertFalse(app.buttons["Search Spotify"].exists)
    XCTAssertFalse(app.buttons["Search Apple Music"].isEnabled)
    XCTAssertFalse(app.buttons["Search YouTube"].isEnabled)
    app.navigationBars["Command Palette"].buttons["Done"].tap()
    artists.tap()
    XCTAssertTrue(app.staticTexts["A Artist"].waitForExistence(timeout: 8))
    app.staticTexts["A Artist"].tap()
    XCTAssertTrue(albumEntry(app, "A Album 1 (2000)").waitForExistence(timeout: 8))
    app.buttons["Command Palette"].firstMatch.tap()
    XCTAssertTrue(app.buttons["Search YouTube"].isEnabled)
    app.buttons["Search YouTube"].tap()
    XCTAssertFalse(app.textFields["Search YouTube"].exists)
    let safari = XCUIApplication(bundleIdentifier: "com.apple.mobilesafari")
    let youtube = XCUIApplication(bundleIdentifier: "com.google.ios.youtube")
    XCTAssertTrue(
      safari.wait(for: .runningForeground, timeout: 3)
        || youtube.wait(for: .runningForeground, timeout: 7))
    app.activate()
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Textamp"].tap()
    app.buttons["Settings…"].tap()
    app.collectionViews.buttons["Library"].tap()
    XCTAssertTrue(app.staticTexts["Show / Hide"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.staticTexts["Library destinations"].exists)
    XCTAssertFalse(app.staticTexts["Show in sidebar"].exists)
    capture(app, "Library Visibility")
  }

  func testAlbumFilteringInListAndGrid() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    app.buttons["sidebar-open-Albums"].tap()
    app.staticTexts["All"].tap()
    for layout in ["List", "Grid"] {
      XCTAssertTrue(albumEntry(app, "A Album 1 (2000)").waitForExistence(timeout: 8))
      app.buttons["View Options"].tap()
      app.buttons[layout].tap()
      app.buttons["column-title"].tap()
      let field = app.textFields.matching(NSPredicate(format: "placeholderValue BEGINSWITH 'Filter '")).firstMatch
      XCTAssertTrue(field.waitForExistence(timeout: 5))
      field.typeText("B Album 2")
      let selected = albumEntry(app, "B Album 2 (2001)")
      XCTAssertTrue(selected.waitForExistence(timeout: 8))
      XCTAssertTrue(albumEntry(app, "A Album 1 (2000)").waitForNonExistence(timeout: 5))
      field.typeText("x")
      XCTAssertTrue(app.staticTexts["No matches"].waitForExistence(timeout: 5))
      field.typeText(XCUIKeyboardKey.delete.rawValue)
      XCTAssertTrue(selected.waitForExistence(timeout: 5))
      selected.tap()
      XCTAssertTrue(app.buttons["Play Album"].waitForExistence(timeout: 8))
      XCTAssertTrue(app.staticTexts["B Album 2"].exists)
      app.buttons["Back"].tap()
      app.buttons["Close Filter"].tap()
    }
    XCTAssertTrue(albumEntry(app, "A Album 1 (2000)").waitForExistence(timeout: 8))
  }

  func testAlbumArtistFilterAndCompactHeader() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    app.buttons["sidebar-open-Albums"].tap()
    app.staticTexts["All"].tap()
    XCTAssertTrue(albumEntry(app, "A Album 1 (2000)").waitForExistence(timeout: 8))
    app.buttons["column-title"].tap()
    let filter = app.textFields.matching(
      NSPredicate(format: "placeholderValue BEGINSWITH 'Filter '")
    ).firstMatch
    XCTAssertTrue(filter.waitForExistence(timeout: 5))
    filter.typeText("\"B Artist\"")
    XCTAssertTrue(albumEntry(app, "B Album 1 (2001)").waitForExistence(timeout: 5))
    XCTAssertTrue(albumEntry(app, "A Album 1 (2000)").waitForNonExistence(timeout: 5))
    XCTAssertEqual(filter.value as? String, "\"B Artist\"")
    capture(app, "Album Filter by Artist")
    albumEntry(app, "B Album 1 (2001)").tap()
    let play = app.buttons["Play Album"]
    XCTAssertTrue(play.waitForExistence(timeout: 5))
    XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5))
    XCTAssertTrue(app.buttons["album-notes"].waitForExistence(timeout: 8))
    play.press(forDuration: 0.8)
    let next = app.buttons.matching(identifier: "Play Album Next")
    let visibleNext = XCTNSPredicateExpectation(
      predicate: NSPredicate { _, _ in next.allElementsBoundByIndex.contains { $0.isHittable } },
      object: nil)
    XCTAssertEqual(XCTWaiter.wait(for: [visibleNext], timeout: 5), .completed)
    XCTAssertTrue(app.buttons["Add Album to Queue"].exists)
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.08, dy: 0.85)).tap()
    XCTAssertTrue(app.staticTexts["B Album 1"].exists)
    XCTAssertLessThan(app.buttons["Add Album to Queue"].frame.midY, play.frame.minY)
    app.swipeUp()
    capture(app, "Detail Section Heading")
  }

  func testLongTrackListIndex() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    app.buttons["Tracks"].tap()
    let table = app.tables["library-column"]
    XCTAssertTrue(table.waitForExistence(timeout: 10))
    let index = table.otherElements["Section index"]
    XCTAssertTrue(index.waitForExistence(timeout: 8))
    let scrollbar = app.descendants(matching: .any)["column-scrollbar"].firstMatch
    XCTAssertTrue(scrollbar.waitForExistence(timeout: 5))
    let nearStart = scrollbar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.05))
    let middle = scrollbar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
    middle.tap()
    XCTAssertNotEqual(scrollbar.value as? String, "0%")
    middle.press(forDuration: 0.5, thenDragTo: nearStart)
    XCTAssertLessThan(Int((scrollbar.value as? String ?? "").dropLast()) ?? 100, 15)
    capture(app, "Long Track List Index")
    index.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.97)).tap()
    let last = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Z Track'"))
      .firstMatch
    XCTAssertTrue(last.waitForExistence(timeout: 5))
    XCTAssertTrue(last.isHittable)
    capture(app, "Track Alphabet Jump")
  }

  func testSystemAppearance() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    connectFixture(app)
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Textamp"].tap()
    app.buttons["Settings…"].tap()
    app.buttons["Appearance"].tap()
    app.buttons["solarized dark"].tap()
    XCTAssertTrue(
      app.buttons.matching(NSPredicate(format: "label == 'solarized dark' AND selected == true"))
        .firstMatch.waitForExistence(timeout: 5))
    app.buttons["system"].tap()
    XCTAssertTrue(
      app.buttons.matching(NSPredicate(format: "label == 'system' AND selected == true"))
        .firstMatch.waitForExistence(timeout: 5))
    capture(app, "System Appearance Settings")
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.buttons["Done"].tap()
    capture(app, "System Library")
    app.buttons["Artists"].tap()
    XCTAssertTrue(app.staticTexts["A Artist"].waitForExistence(timeout: 8))
    capture(app, "System Artists")
  }

  func testCommandPaletteFindsTypoAndHiddenSynonym() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    if app.buttons["Add Library"].waitForExistence(timeout: 2) { connectFixture(app) }
    dismissIconNotice(XCUIApplication(bundleIdentifier: "com.apple.springboard"), timeout: 2)

    for query in ["settigns", "preferences"] {
      app.buttons["Command Palette"].firstMatch.tap()
      let field = app.textFields["Find a command"]
      XCTAssertTrue(field.waitForExistence(timeout: 5))
      field.tap()
      field.typeText(query)
      let settings = app.buttons["palette-command:Settings…:OpenSettings"]
      XCTAssertTrue(settings.waitForExistence(timeout: 5))
      app.navigationBars["Command Palette"].buttons["Done"].tap()
    }
  }

  func testPaletteMenuHierarchy() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    connectFixture(app)
    app.buttons["Command Palette"].firstMatch.tap()
    let view = app.buttons["palette-group:View"]
    XCTAssertTrue(view.waitForExistence(timeout: 5))
    XCTAssertTrue(app.buttons["palette-group:Textamp"].exists)
    view.tap()
    XCTAssertTrue(app.navigationBars["View"].waitForExistence(timeout: 5))
    app.buttons["palette-group:View/Browse"].tap()
    XCTAssertTrue(app.navigationBars["Browse"].waitForExistence(timeout: 5))
    app.navigationBars["Browse"].buttons["Back"].tap()
    XCTAssertTrue(app.navigationBars["View"].waitForExistence(timeout: 5))
    app.buttons["palette-group:View/Browse"].tap()
    app.buttons["palette-command:Play Queue:GotoQueue"].tap()
    XCTAssertTrue(app.buttons["bottom-queue"].isSelected)
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Playback"].tap()
    XCTAssertTrue(app.navigationBars["Playback"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["palette-command:Play/Pause:PlayPause"].isEnabled)
    capture(app, "Palette Playback Menu")
    let field = app.textFields["Find a command"]
    field.tap()
    field.typeText("Settings")
    app.buttons["palette-command:Settings…:OpenSettings"].tap()
    XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
  }

  func testSwitchLibraryMenuAndSheet() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=selector-menu-validation"]
    app.launch()
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Textamp"].tap()
    XCTAssertTrue(app.buttons["palette-command:Refresh Library:Refresh"].exists)
    XCTAssertTrue(app.buttons["palette-command:Settings…:OpenSettings"].exists)
    app.buttons["palette-command:Switch Library…:SwitchLibrary"].tap()
    XCTAssertTrue(app.navigationBars["Switch Library"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.toolbars.buttons["Command Palette"].exists)
    app.navigationBars["Switch Library"].buttons["Done"].tap()
  }

  func testPaletteDestinations() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    connectFixture(app)
    let miniPlayer = app.otherElements["mini-player"]
    XCTAssertTrue(app.buttons["Now Playing, Nothing playing"].exists)
    XCTAssertFalse(miniPlayer.sliders.firstMatch.exists)
    capture(app, "Idle Mini Player")
    runCommand(app, query: "Play Queue", label: "Play Queue")
    XCTAssertTrue(app.navigationBars["Play Queue"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.buttons["bottom-queue"].isSelected)
    runCommand(app, query: "Now Playing", label: "Now Playing")
    XCTAssertTrue(app.buttons["Close Now Playing"].waitForExistence(timeout: 5))
    runCommand(app, query: "Play Queue", label: "Play Queue")
    XCTAssertTrue(app.navigationBars["Play Queue"].waitForExistence(timeout: 5))
    runCommand(app, query: "Library", label: "Library")
    XCTAssertTrue(app.staticTexts["A Artist"].waitForExistence(timeout: 8))
    runCommand(app, query: "View Options", label: "View Options…")
    XCTAssertTrue(app.navigationBars["View Options"].waitForExistence(timeout: 5))
    XCTAssertTrue(
      app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Reverse order'")).firstMatch
        .exists
    )
    app.buttons["Done"].tap()
    runCommand(app, query: "Filter Column", label: "Filter Column…")
    let filter = app.textFields.matching(
      NSPredicate(format: "placeholderValue BEGINSWITH 'Filter '")
    ).firstMatch
    XCTAssertTrue(filter.waitForExistence(timeout: 5))
    app.buttons["Close Filter"].tap()
    runCommand(app, query: "Switch Library", label: "Switch Library…")
    XCTAssertTrue(app.navigationBars["Switch Library"].waitForExistence(timeout: 5))
    app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Fixture Music'")).firstMatch.tap()
    XCTAssertTrue(app.staticTexts["A Artist"].waitForExistence(timeout: 5))
    runCommand(app, query: "Artist Mix", label: "Artist…")
    XCTAssertTrue(app.textFields["Find artists"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["Play Station"].isEnabled)
    app.buttons["A Artist"].tap()
    XCTAssertTrue(app.buttons["Play Station"].isEnabled)
    capture(app, "Artist Mix Picker")
    app.buttons["Done"].tap()
  }

  func testRadioArtistMixTakesMultipleArtists() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    let radio = app.buttons["sidebar-open-Radio"]
    for _ in 0..<4 where !radio.isHittable { app.swipeUp() }
    radio.tap()
    app.buttons["Artist Mix"].tap()
    let play = app.buttons["Play Mix"]
    let field = app.textFields["Find artists"]
    XCTAssertTrue(field.waitForExistence(timeout: 8))
    XCTAssertFalse(play.isEnabled)
    func add(_ name: String) {
      field.tap()
      field.typeText(name)
      let result = app.buttons[name]
      XCTAssertTrue(result.waitForExistence(timeout: 5), app.debugDescription)
      result.tap()
      XCTAssertTrue(app.buttons["Remove \(name)"].waitForExistence(timeout: 5))
    }
    add("A Artist")
    XCTAssertTrue(play.isEnabled)
    add("B Artist")
    field.tap()
    field.typeText("C Artist")
    XCTAssertTrue(app.buttons["C Artist"].waitForExistence(timeout: 5))
    app.buttons["Remove A Artist"].tap()
    XCTAssertTrue(app.buttons["Remove A Artist"].waitForNonExistence(timeout: 5))
    XCTAssertTrue(app.buttons["Remove B Artist"].exists)
    app.buttons["Clear search"].tap()
    add("A Artist")
    play.tap()
    let queueType = app.buttons["Queue Type"].firstMatch
    XCTAssertTrue(queueType.waitForExistence(timeout: 8))
    XCTAssertEqual(queueType.value as? String, "Artist Mix")
    XCTAssertTrue(app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Play A Track' OR label BEGINSWITH 'Play B Track'")).firstMatch.waitForExistence(timeout: 8))
  }

  func testLibraryRadioRemembersStationAndOpensItsInput() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    let radio = app.buttons["library-radio"]
    XCTAssertTrue(radio.waitForExistence(timeout: 8))
    if radio.value as? String != "Play Random Albums" {
      radio.press(forDuration: 0.8)
      app.buttons["Play Random Albums"].firstMatch.tap()
    }
    XCTAssertEqual(radio.value as? String, "Play Random Albums")
    radio.press(forDuration: 0.8)
    app.buttons["Decade Mix"].firstMatch.tap()
    XCTAssertEqual(radio.value as? String, "Decade Mix")
    radio.tap()
    XCTAssertTrue(app.buttons["Back"].waitForExistence(timeout: 8))
    XCTAssertTrue(app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "2000")).firstMatch.waitForExistence(timeout: 8))
  }

  func testPlaylistFilesUseSystemPickers() {
    continueAfterFailure = false
    let app = openAlbumDetails("A Album 1 (2000)")
    app.buttons["Add Album to Queue"].tap()
    runCommand(app, query: "Export Queue as CSV", label: "CSV…")
    let save = app.buttons["Save"].firstMatch
    XCTAssertTrue(save.waitForExistence(timeout: 12), app.debugDescription)
    save.tap()
    let system = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    let replace = system.alerts.buttons["Replace"]
    if replace.waitForExistence(timeout: 4) { replace.tap() }
    XCTAssertTrue(app.buttons["bottom-library"].waitForExistence(timeout: 10))
    runCommand(app, query: "Import Playlist", label: "Import Playlist…")
    let file = app.descendants(matching: .any).matching(NSPredicate(
      format: "label BEGINSWITH 'Queue' AND (elementType == %d OR elementType == %d)",
      XCUIElement.ElementType.cell.rawValue, XCUIElement.ElementType.button.rawValue)).firstMatch
    XCTAssertTrue(file.waitForExistence(timeout: 12), app.debugDescription)
    file.tap()
    XCTAssertTrue(app.buttons["bottom-library"].waitForExistence(timeout: 10))
    let playlists = app.buttons["sidebar-open-Playlists"]
    if !playlists.exists { app.buttons["bottom-library"].tap() }
    playlists.tap()
    let imported = app.tables["library-column"].staticTexts["Queue"]
    XCTAssertTrue(imported.waitForExistence(timeout: 10), app.debugDescription)
    imported.tap()
    XCTAssertTrue(app.tables["library-column"].staticTexts["A Track 1.01"].waitForExistence(timeout: 8))
  }

  func testSavedQueueOpensFromPlaylists() {
    let app = openAlbumDetails("A Album 1 (2000)")
    app.buttons["Add Album to Queue"].tap()
    app.buttons["bottom-queue"].tap()
    runCommand(app, query: "Save Queue as Playlist", label: "Save Queue as Playlist…")
    let field = app.textFields["Playlist Name"]
    XCTAssertTrue(field.waitForExistence(timeout: 5))
    field.tap()
    field.typeText("Saved Queue Tracks")
    app.buttons["Save Playlist"].tap()
    XCTAssertTrue(field.waitForNonExistence(timeout: 5))
    app.buttons["bottom-library"].tap()
    let playlists = app.buttons["sidebar-open-Playlists"]
    if !playlists.exists { app.buttons["bottom-library"].tap() }
    XCTAssertTrue(playlists.waitForExistence(timeout: 5))
    playlists.tap()
    let saved = app.tables["library-column"].staticTexts["Saved Queue Tracks"]
    XCTAssertTrue(saved.waitForExistence(timeout: 8))
    saved.tap()
    XCTAssertTrue(app.buttons["playlist-edit"].waitForExistence(timeout: 8))
    XCTAssertTrue(app.tables["library-column"].staticTexts["A Track 1.01"].waitForExistence(timeout: 8))
  }

  func testPlaylistDraftSurvivesPalette() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    runCommand(app, query: "Library", label: "Library")
    XCTAssertTrue(app.staticTexts["A Artist"].waitForExistence(timeout: 8))
    app.staticTexts["A Artist"].tap()
    XCTAssertTrue(albumEntry(app, "A Album 1 (2000)").waitForExistence(timeout: 8))
    albumEntry(app, "A Album 1 (2000)").tap()
    XCTAssertTrue(app.buttons["Add Album to Queue"].waitForExistence(timeout: 8))
    app.buttons["Add Album to Queue"].tap()
    runCommand(app, query: "Save Queue as Playlist", label: "Save Queue as Playlist…")
    let name = app.textFields["Playlist Name"]
    XCTAssertTrue(name.waitForExistence(timeout: 5))
    name.tap()
    name.typeText("Evening")
    checkPalette(app)
    XCTAssertTrue(name.waitForExistence(timeout: 5))
    XCTAssertEqual(name.value as? String, "Evening")
    capture(app, "Restored Playlist Draft")
  }

  func testCacheRefreshKeepsBrowsingAndNativePlaybackResponsive() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launch()
    XCTAssertTrue(app.buttons["Artists"].waitForExistence(timeout: 5))
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Textamp"].tap()
    app.buttons["Settings…"].tap()
    XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
    app.buttons["Caches"].tap()
    XCTAssertTrue(app.navigationBars["Caches"].waitForExistence(timeout: 5))
    let cancel = app.buttons["Cancel scan"]
    if cancel.exists { cancel.tap() }
    let scan = app.buttons["Re-scan library now"]
    XCTAssertTrue(scan.waitForExistence(timeout: 5))
    scan.tap()
    XCTAssertTrue(cancel.waitForExistence(timeout: 3))
    capture(app, "Cache Management During Refresh")
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.buttons["Done"].tap()
    XCTAssertTrue(app.buttons["Artists"].waitForExistence(timeout: 3))
    app.buttons["Artists"].tap()
    XCTAssertTrue(app.staticTexts["A Artist"].waitForExistence(timeout: 3))
    app.staticTexts["A Artist"].tap()
    XCTAssertTrue(albumEntry(app, "A Album 1 (2000)").waitForExistence(timeout: 3))
    albumEntry(app, "A Album 1 (2000)").tap()
    XCTAssertTrue(app.buttons["Play Album"].waitForExistence(timeout: 3))
    app.buttons["Play Album"].tap()
    XCTAssertTrue(app.buttons["Pause"].firstMatch.waitForExistence(timeout: 3))
    capture(app, "Playback During Cache Refresh")
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Textamp"].tap()
    app.buttons["Settings…"].tap()
    XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
    app.buttons["Caches"].tap()
    XCTAssertTrue(app.navigationBars["Caches"].waitForExistence(timeout: 5))
    XCTAssertTrue(
      cancel.waitForExistence(timeout: 3),
      "Browsing and playback must finish before the delayed scan")
    cancel.tap()
    XCTAssertTrue(scan.waitForExistence(timeout: 3))
  }

  func testSearchLyricsVisibilitySwitchAtEndOfLibrarySettings() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=lyrics-preference"]
    app.launch()
    connectFixture(app, audioMuse: true)
    runCommand(app, query: "Settings", label: "Settings…")
    XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
    let library = app.buttons["settings-section-Library"]
    library.tap()
    if !app.navigationBars["Library"].waitForExistence(timeout: 2), library.isHittable { library.tap() }
    XCTAssertTrue(app.navigationBars["Library"].waitForExistence(timeout: 5), app.debugDescription)
    let toggle = app.switches["preference-Search lyrics…"]
    for _ in 0..<8 where !toggle.isHittable { app.swipeUp() }
    XCTAssertTrue(toggle.isHittable, app.debugDescription)
    let previous = toggle.value as? String
    toggle.tap()
    let changed = NSPredicate(format: "value != %@", previous ?? "")
    expectation(for: changed, evaluatedWith: toggle)
    waitForExpectations(timeout: 4)
  }

  func testSidebarPreferencesPersist() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Textamp"].tap()
    app.buttons["Settings…"].tap()
    XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
    app.collectionViews.buttons["Library"].tap()
    XCTAssertTrue(app.staticTexts["library-heading"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.staticTexts["Singles: maximum tracks"].exists)
    XCTAssertTrue(app.steppers.firstMatch.exists)
    let folders = app.switches["preference-Folders"]
    for _ in 0..<5 where !folders.isHittable { app.swipeUp() }
    XCTAssertTrue(folders.exists)
    XCTAssertEqual(folders.value as? String, "1")
    folders.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)).tap()
    expectation(for: NSPredicate(format: "value == '0'"), evaluatedWith: folders)
    waitForExpectations(timeout: 5)
    capture(app, "Sidebar Preferences")
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.collectionViews.buttons["Library"].tap()
    XCTAssertTrue(app.staticTexts["library-heading"].waitForExistence(timeout: 5))
    for _ in 0..<5 where !folders.isHittable { app.swipeUp() }
    XCTAssertEqual(folders.value as? String, "0")
    folders.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)).tap()
  }

  private func runCommand(_ app: XCUIApplication, query: String, label: String) {
    app.buttons["Command Palette"].firstMatch.tap()
    let field = app.textFields["Find a command"]
    XCTAssertTrue(field.waitForExistence(timeout: 5))
    field.tap()
    field.typeText(query)
    let command = app.buttons.matching(
      NSPredicate(format: "identifier BEGINSWITH %@", "palette-command:\(label):")
    ).firstMatch
    XCTAssertTrue(command.waitForExistence(timeout: 5))
    command.tap()
  }

  func testPaletteSettingsSearchAndDiscovery() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    connectFixture(app)
    runCommand(app, query: "Settings", label: "Settings…")
    XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
    app.buttons["Appearance"].tap()
    runCommand(app, query: "Play Queue", label: "Play Queue")
    XCTAssertTrue(app.navigationBars["Play Queue"].waitForExistence(timeout: 5))
    runCommand(app, query: "Search", label: "Search…")
    let field = app.textFields["Search your library"]
    XCTAssertTrue(field.waitForExistence(timeout: 5))
    field.tap()
    field.typeText("A Track 1.03")
    let result = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'A Track 1.03'"))
      .firstMatch
    XCTAssertTrue(result.waitForExistence(timeout: 8))
    result.tap()
    XCTAssertTrue(app.staticTexts["Details"].waitForExistence(timeout: 8))
    for label in ["Play Track", "Play Next", "Add to Queue"] {
      let button = app.buttons[label]
      XCTAssertTrue(button.exists)
      XCTAssertFalse(button.staticTexts.firstMatch.exists)
    }
    capture(app, "Track Playback Icons")
    XCTAssertTrue(app.staticTexts["Details"].waitForExistence(timeout: 8))
    XCTAssertFalse(app.buttons["Done"].exists)
    app.swipeUp()
    let similar = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'B Track'"))
      .firstMatch
    XCTAssertTrue(similar.waitForExistence(timeout: 8))
    capture(app, "Similar Tracks in Details")
    similar.tap()
    XCTAssertTrue(app.staticTexts["Details"].waitForExistence(timeout: 8))
    app.buttons["B Album 1"].firstMatch.tap()
    XCTAssertTrue(app.buttons["Play Album"].waitForExistence(timeout: 8))
    XCTAssertTrue(app.buttons["Play Album"].exists)
    let related = app.tables["library-column"].buttons["A Artist — A Album 1"]
    for _ in 0..<8 where !related.isHittable { app.tables["library-column"].swipeUp() }
    XCTAssertTrue(related.isHittable, app.debugDescription)
    related.tap()
    XCTAssertTrue(app.staticTexts["Details"].waitForExistence(timeout: 8))
    let table = app.tables["library-column"]
    let artist = table.buttons["B Artist"]
    for _ in 0..<10 {
      if artist.exists && !artist.frame.isEmpty && table.frame.insetBy(dx: 0, dy: 24).contains(CGPoint(x: artist.frame.midX, y: artist.frame.midY)) { break }
      table.swipeUp(velocity: .slow)
    }
    XCTAssertTrue(artist.waitForExistence(timeout: 8))
    capture(app, "Related Artists in Biography")
    artist.tap()
    XCTAssertTrue(albumEntry(app, "B Album 1 (2001)").waitForExistence(timeout: 8))
  }

  func testArtistGoogleSearchOpensBrowser() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    app.buttons["Artists"].tap()
    let artist = app.staticTexts["A Artist"]
    XCTAssertTrue(artist.waitForExistence(timeout: 8))
    artist.tap()
    XCTAssertFalse(app.staticTexts["Artist Details"].exists)
    albumEntry(app, "A Album 1 (2000)").tap()
    app.buttons["Artist Biography"].tap()
    let search = app.buttons["Search Google"]
    XCTAssertTrue(search.waitForExistence(timeout: 8))
    search.tap()
    let safari = XCUIApplication(bundleIdentifier: "com.apple.mobilesafari")
    XCTAssertTrue(safari.wait(for: .runningForeground, timeout: 10))
  }

  private func albumEntry(_ app: XCUIApplication, _ title: String) -> XCUIElement {
    app.descendants(matching: .any).matching(NSPredicate(format: "label == %@ AND (elementType == %d OR elementType == %d)", title, XCUIElement.ElementType.button.rawValue, XCUIElement.ElementType.staticText.rawValue)).firstMatch
  }

  private func openAlbumDetails(_ album: String, artist: String = "A Artist", profile: String = "fixture-validation", silent: Bool = true) -> XCUIApplication {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--test-profile=\(profile)"] + (silent ? ["--silent"] : [])
    app.launch()
    connectFixture(app)
    app.buttons["Artists"].tap()
    XCTAssertTrue(app.staticTexts[artist].waitForExistence(timeout: 8))
    app.staticTexts[artist].tap()
    albumEntry(app, album).tap()
    return app
  }

  func testRandomAlbumOpensDetails() {
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    runCommand(app, query: "Go to Random Album", label: "Go to Random Album")
    XCTAssertTrue(app.buttons["Play Album"].waitForExistence(timeout: 8))
    if UIDevice.current.userInterfaceIdiom == .phone {
      XCTAssertEqual(app.buttons["column-title"].frame.midY, app.buttons["Back"].frame.midY, accuracy: 2)
    }
    XCTAssertTrue(app.buttons["Play Album Next"].exists)
  }

  #if targetEnvironment(simulator)
  func testAppleMusicKeyboardShortcutReportsMissingApp() {
    let app = openAlbumDetails("A Album 1 (2000)")
    XCTAssertTrue(app.buttons["Play Album"].waitForExistence(timeout: 8))
    app.typeKey("a", modifierFlags: [.command, .shift])
    XCTAssertTrue(app.alerts.staticTexts["Could not open Apple Music."].waitForExistence(timeout: 8))
    XCTAssertEqual(app.state, .runningForeground)
  }
  #endif

  func testTrackDetailShowsInlineLyricsAndSimilarTracks() {
    let app = openAlbumDetails("A Album 1 (2000)")
    let table = app.tables.containing(.button, identifier: "Play Album").firstMatch
    let track = table.staticTexts["A Track 1.01"]
    if !track.isHittable { table.swipeUp(velocity: .slow) }
    track.tap()
    XCTAssertTrue(app.staticTexts["Synthetic fixture lyrics.\nSecond fixture line."].waitForExistence(timeout: 10))
    XCTAssertTrue(app.staticTexts["Similar Tracks"].exists)
    capture(app, "Textamp Track Details")
    XCTAssertFalse(app.images["track-detail-artwork"].exists)
  }

  func testAlbumDetailsShowsNavidromeNotes() {
    let app = openAlbumDetails("A Album 1 (2000)")
    XCTAssertTrue(app.buttons["Play Album"].waitForExistence(timeout: 8))
    XCTAssertTrue(app.buttons["Play Album Next"].exists)
    XCTAssertTrue(app.buttons["Download Album"].exists)
    XCTAssertTrue(app.buttons["A Artist"].exists)
    XCTAssertTrue(app.staticTexts["A Album 1"].exists)
    XCTAssertFalse(app.staticTexts["album-review"].exists)
    if UIDevice.current.userInterfaceIdiom == .phone {
      XCTAssertEqual(app.staticTexts["A Album 1"].firstMatch.frame.midX, app.frame.midX, accuracy: 1)
    }
    XCTAssertLessThan(app.buttons["Artist Biography"].frame.midX, app.buttons["Play Album"].frame.midX)
    XCTAssertGreaterThan(app.buttons["album-notes"].frame.midX, app.buttons["Play Album"].frame.midX)
    XCTAssertFalse(app.buttons["Shuffle Album"].exists)
    XCTAssertEqual(app.buttons["album-notes"].frame.midY, app.buttons["Play Album"].frame.midY, accuracy: 1)
    XCTAssertEqual(app.buttons["Play Album Next"].frame.midY, app.buttons["Download Album"].frame.midY, accuracy: 1)
    XCTAssertEqual(app.buttons["Play Album"].frame.midX, app.staticTexts["A Album 1"].firstMatch.frame.midX, accuracy: 1)
    capture(app, "Textamp Album Page")
    XCTAssertFalse(app.buttons["View Options"].exists)
    let next = app.buttons["Play Album Next"]
    if !next.isHittable { app.tables["library-column"].swipeUp(velocity: .slow) }
    XCTAssertTrue(next.isHittable)
    XCTAssertTrue(app.buttons["Add Album to Queue"].isHittable)
    let notes = app.buttons["album-notes"]
    if !notes.isHittable { app.tables["library-column"].swipeUp() }
    notes.tap()
    let review = app.staticTexts["album-review"]
    XCTAssertTrue(review.waitForExistence(timeout: 8))
    let rendered = XCTAttachment(string: review.label)
    rendered.name = "Rendered album review"
    rendered.lifetime = .keepAlways
    add(rendered)
    XCTAssertTrue(review.label.hasSuffix("\n\nSource: Apple Music"))
    XCTAssertFalse(app.descendants(matching: .any)["album-source-link"].exists)
  }

  func testAlbumScrollsFromTracksToSimilarAlbums() {
    let orientation = XCUIDevice.shared.orientation
    if UIDevice.current.userInterfaceIdiom == .pad { XCUIDevice.shared.orientation = .landscapeLeft }
    defer { XCUIDevice.shared.orientation = orientation }
    let app = openAlbumDetails("A Album 1 (2000)")
    let table = app.tables.containing(.button, identifier: "Play Album").firstMatch
    XCTAssertTrue(table.waitForExistence(timeout: 8))
    if UIDevice.current.userInterfaceIdiom == .pad { capture(app, "Textamp iPad Album Page") }
    let track = table.cells.matching(NSPredicate(format: "identifier ENDSWITH 'song-0-0-4'")).firstMatch
    table.swipeUp(velocity: .slow)
    for _ in 0..<4 where !table.frame.contains(CGPoint(x: track.frame.midX, y: track.frame.midY)) { table.swipeUp(velocity: .slow) }
    XCTAssertTrue(table.frame.contains(CGPoint(x: track.frame.midX, y: track.frame.midY)))
    capture(app, "Textamp Album Tracks")
    let related = table.buttons["B Artist — B Album 1"]
    for _ in 0..<8 where !related.isHittable { table.swipeUp(velocity: .slow) }
    XCTAssertTrue(related.isHittable)
    capture(app, "Textamp Similar Albums")
  }

  func testSimilarAlbumOpensAlbumPage() {
    let app = openAlbumDetails("A Album 1 (2000)")
    let tracks = app.tables["library-column"]
    let related = tracks.buttons["B Artist — B Album 1"]
    for _ in 0..<5 where !related.isHittable { tracks.swipeUp() }
    XCTAssertTrue(related.isHittable, app.debugDescription)
    related.tap()
    XCTAssertTrue(app.staticTexts["B Album 1"].waitForExistence(timeout: 8))
    XCTAssertTrue(app.buttons["Play Album"].exists)
  }

  func testNowPlayingScrollsToMetadataAndSimilarTracks() throws {
    try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .phone)
    let app = openAlbumDetails("A Album 1 (2000)")
    XCTAssertFalse(app.navigationBars.buttons.matching(NSPredicate(format: "label BEGINSWITH[c] %@", "Filter")).firstMatch.exists)
    let back = app.navigationBars.buttons["Back"]
    for name in ["Play Album Next", "Add Album to Queue"] {
      let button = app.navigationBars.buttons[name]
      XCTAssertTrue(button.waitForExistence(timeout: 5) && button.isHittable, name)
      XCTAssertEqual(button.frame.midY, back.frame.midY, accuracy: 2)
    }
    let download = app.navigationBars.buttons["Download Album"]
    if download.isHittable {
      XCTAssertEqual(download.frame.midY, back.frame.midY, accuracy: 2)
    } else {
      let overflow = app.navigationBars.buttons["More"]
      XCTAssertTrue(overflow.isHittable)
      overflow.tap()
      XCTAssertTrue(app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "Download Album")).firstMatch.waitForExistence(timeout: 4), app.debugDescription)
      app.coordinate(withNormalizedOffset: CGVector(dx: 0.1, dy: 0.65)).tap()
    }
    app.buttons["Play Album"].tap()
    openNowPlaying(app)
    let scroll = app.scrollViews["playing-scroll"]
    scroll.buttons["Next Track"].tap()
    XCTAssertTrue(scroll.buttons["A Track 1.02"].waitForExistence(timeout: 8), app.debugDescription)
    scroll.swipeUp()
    XCTAssertTrue(app.staticTexts["Synthetic fixture lyrics.\nSecond fixture line."].waitForExistence(timeout: 10))
    let similar = app.staticTexts["Similar Tracks"]
    for _ in 0..<4 where !similar.isHittable { scroll.swipeUp() }
    XCTAssertTrue(similar.isHittable)
    XCTAssertFalse(scroll.descendants(matching: .any)["now-playing-artwork"].isHittable)
    let recommendation = scroll.buttons.matching(NSPredicate(format: "label CONTAINS ' — '")).firstMatch
    for _ in 0..<3 where !recommendation.isHittable { scroll.swipeUp() }
    XCTAssertTrue(recommendation.isHittable)
    scroll.swipeDown()
    XCTAssertTrue(app.buttons["Close Now Playing"].isHittable)
  }

  func testAlbumQueueButtonsWorkWithoutStartingPlayback() {
    let app = openAlbumDetails("A Album 1 (2000)")
    let append = app.buttons["Add Album to Queue"]
    XCTAssertTrue(append.waitForExistence(timeout: 8))
    if !append.isHittable { app.tables["library-column"].swipeUp(velocity: .slow) }
    append.tap()
    app.buttons["Play Album Next"].tap()
    app.buttons["bottom-queue"].tap()
    XCTAssertTrue(app.staticTexts["A Track 1.01"].firstMatch.waitForExistence(timeout: 8))
  }

  func testAlbumPlaysWithInlineWaveform() {
    let app = openAlbumDetails("A Album 1 (2000)")
    app.buttons["Play Album"].tap()
    openNowPlaying(app)
    let slider = app.sliders["waveform-position"]
    XCTAssertTrue(slider.waitForExistence(timeout: 8))
    let art = app.descendants(matching: .any)["now-playing-artwork"]
    XCTAssertTrue(art.exists)
    XCTAssertEqual(art.frame.midX, slider.frame.midX, accuracy: 1)
    XCTAssertEqual(art.frame.width, art.frame.height, accuracy: 1)
    let volume = app.sliders["Volume"]
    XCTAssertTrue(volume.isHittable)
    XCTAssertLessThanOrEqual(volume.frame.maxY, app.buttons["bottom-library"].frame.minY)
    let initialArtwork = art.frame
    capture(app, "Centered Now Playing")
    XCTAssertTrue(slider.isHittable)
    XCTAssertFalse(app.buttons["Expand Waveform"].exists)
    slider.adjust(toNormalizedSliderPosition: 0.5)
    let waveform = app.descendants(matching: .any)["playback-waveform"]
    let ready = expectation(for: NSPredicate { _, _ in
      (waveform.value as? String)?.contains(":") == true
    }, evaluatedWith: app)
    wait(for: [ready], timeout: 15)
    XCTAssertEqual(art.frame, initialArtwork)
    XCTAssertTrue(app.buttons["now-playing-toggle"].isHittable)
  }

  func testAlbumDetailsOmitsAppleMusicLinkWithoutReview() {
    let app = openAlbumDetails("A Album 2 (2000)")
    let source = app.descendants(matching: .any)["album-source-link"]
    XCTAssertTrue(app.buttons["Play Album"].waitForExistence(timeout: 8))
    XCTAssertFalse(source.exists)
    XCTAssertFalse(app.staticTexts["album-review"].exists)
  }

  func testAlbumDetailsReceivesLateNotesWithIndependentCredit() {
    let app = openAlbumDetails("B Album 1 (2001)", artist: "B Artist")
    let source = app.descendants(matching: .any)["album-source-link"]
    let notes = app.buttons["album-notes"]
    XCTAssertTrue(notes.waitForExistence(timeout: 12))
    notes.tap()
    let review = app.staticTexts["album-review"]
    XCTAssertTrue(review.waitForExistence(timeout: 12))
    XCTAssertTrue(review.label.hasSuffix("\n\nSource: Wikipedia"))
    XCTAssertFalse(source.exists)
  }

  func testArtistAlbumGridOpensTheChosenRelease() {
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    app.buttons["Artists"].tap()
    app.staticTexts["B Artist"].firstMatch.tap()
    let first = app.buttons["B Album 1 (2001)"]
    let second = albumEntry(app, "B Album 2 (2001)")
    XCTAssertTrue(second.waitForExistence(timeout: 8))
    XCTAssertEqual(first.frame.minY, second.frame.minY, accuracy: 1)
    XCTAssertLessThan(first.frame.midX, second.frame.midX)
    XCTAssertEqual(first.frame.width, second.frame.width, accuracy: 1)
    if UIDevice.current.userInterfaceIdiom == .phone && app.frame.width < 418 {
      XCTAssertGreaterThan(second.frame.maxX, app.frame.width * 0.8)
    }
    capture(app, "Textamp Artist Album Grid")
    app.buttons["View Options"].tap()
    app.buttons["List"].tap()
    let listRow = app.staticTexts["B Album 1 (2001)"]
    XCTAssertTrue(listRow.waitForExistence(timeout: 5))
    XCTAssertFalse(first.exists)
    app.buttons["View Options"].tap()
    app.buttons["Grid"].tap()
    XCTAssertTrue(first.waitForExistence(timeout: 5))
    second.tap()
    XCTAssertTrue(app.buttons["Play Album"].waitForExistence(timeout: 8))
    XCTAssertTrue(app.staticTexts["B Album 2"].exists)
  }

  func testAlbumOpensBiographyAsSoonAsItIsAvailable() {
    let app = openAlbumDetails("A Album 1 (2000)")
    let header = app.tables.containing(.button, identifier: "Play Album").firstMatch
    let biography = header.buttons["Artist Biography"]
    XCTAssertTrue(biography.waitForExistence(timeout: 10))
    biography.tap()
    XCTAssertTrue(app.staticTexts["A synthetic artist biography for native UI testing."].waitForExistence(timeout: 5))
  }

  func testAlbumDetailsRelatedArtists() {
    let app = openAlbumDetails("A Album 1 (2000)")
    let table = app.tables.containing(.button, identifier: "Play Album").firstMatch
    let artist = table.buttons["B Artist"]
    for _ in 0..<10 {
      if artist.exists && !artist.frame.isEmpty && table.frame.insetBy(dx: 0, dy: 24).contains(CGPoint(x: artist.frame.midX, y: artist.frame.midY)) { break }
      table.swipeUp(velocity: .slow)
    }
    XCTAssertTrue(artist.waitForExistence(timeout: 8))
    XCTAssertTrue(artist.isHittable)
    capture(app, "Textamp Album Recommendations")
    artist.tap()
    XCTAssertTrue(albumEntry(app, "B Album 1 (2001)").waitForExistence(timeout: 8))
    XCTAssertFalse(app.staticTexts["Artist Details"].exists)
    let preview = app.staticTexts["artist-bio-preview"]
    let biography = "A synthetic artist biography for native UI testing."
    let loaded = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", biography), object: preview)
    XCTAssertEqual(XCTWaiter.wait(for: [loaded], timeout: 8), .completed)
    let artistTable = app.tables.containing(.staticText, identifier: "artist-bio-preview").firstMatch
    let relatedArtist = artistTable.buttons["C Artist"]
    for _ in 0..<6 {
      if relatedArtist.exists && relatedArtist.isHittable && !relatedArtist.frame.isEmpty && artistTable.frame.insetBy(dx: 0, dy: 12).contains(CGPoint(x: relatedArtist.frame.midX, y: relatedArtist.frame.midY)) { break }
      artistTable.swipeUp(velocity: .slow)
    }
    XCTAssertTrue(relatedArtist.isHittable)
    capture(app, "Textamp Artist Recommendations")
    for _ in 0..<6 {
      if app.buttons["Artist Biography"].exists && app.buttons["Artist Biography"].isHittable { break }
      artistTable.swipeDown(velocity: .slow)
    }
    capture(app, "Textamp Artist Albums")
    app.buttons["Artist Biography"].tap()
    XCTAssertTrue(app.staticTexts[biography].waitForExistence(timeout: 5))
    XCTAssertTrue(app.navigationBars.buttons["Search Google"].exists)

  }

  func testLibraryDetailsQueueAndDownloads() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    connectFixture(app)
    XCTAssertTrue(app.buttons["Artists"].waitForExistence(timeout: 10))
    app.buttons["Artists"].tap()
    XCTAssertTrue(app.staticTexts["A Artist"].waitForExistence(timeout: 10))
    app.staticTexts["A Artist"].tap()
    XCTAssertTrue(albumEntry(app, "A Album 1 (2000)").waitForExistence(timeout: 10))
    albumEntry(app, "A Album 1 (2000)").tap()
    XCTAssertTrue(app.buttons["Play Album"].waitForExistence(timeout: 10))
    XCTAssertTrue(app.buttons["Play Album"].waitForExistence(timeout: 8))
    app.buttons["Play Album"].tap()
    XCTAssertTrue(app.buttons["bottom-queue"].waitForExistence(timeout: 5))
    app.buttons["bottom-queue"].tap()
    XCTAssertTrue(app.staticTexts["A Track 1.01"].waitForExistence(timeout: 8))
    let first = app.staticTexts["A Track 1.01"].firstMatch
    let third = app.staticTexts["A Track 1.03"].firstMatch
    first.press(forDuration: 0.7, thenDragTo: third)
    XCTAssertLessThan(app.staticTexts["A Track 1.02"].firstMatch.frame.minY, first.frame.minY)
    app.staticTexts["A Track 1.02"].firstMatch.swipeLeft()
    app.buttons["Remove"].firstMatch.tap()
    XCTAssertFalse(app.staticTexts["A Track 1.02"].exists)
    app.buttons["Queue Tools"].tap()
    app.buttons.matching(NSPredicate(format: "label CONTAINS[c] 'Save'")).firstMatch.tap()
    XCTAssertTrue(app.textFields["Playlist Name"].waitForExistence(timeout: 5))
    app.textFields["Playlist Name"].tap()
    app.textFields["Playlist Name"].typeText("Phone Queue")
    app.buttons["Save Playlist"].tap()
    let dismissed = NSPredicate(format: "exists == false")
    expectation(for: dismissed, evaluatedWith: app.textFields["Playlist Name"])
    waitForExpectations(timeout: 5)
    openNowPlaying(app)
    app.buttons["Command Palette"].tap()
    let field = app.textFields["Find a command"]
    XCTAssertTrue(field.waitForExistence(timeout: 5))
    field.tap()
    field.typeText("Download Play Queue")
    XCTAssertTrue(app.buttons["Download Play Queue"].waitForExistence(timeout: 5))
    app.buttons["Download Play Queue"].tap()
    app.buttons["Command Palette"].tap()
    field.tap()
    field.typeText("Manage Downloads")
    XCTAssertTrue(app.buttons["Manage Downloads"].waitForExistence(timeout: 5))
    app.buttons["Manage Downloads"].tap()
    XCTAssertTrue(app.navigationBars["Downloads"].waitForExistence(timeout: 10))
    XCTAssertTrue(
      app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH '14 tracks'")).firstMatch
        .waitForExistence(timeout: 15))
    capture(app, "Downloads")
    app.buttons["Toggle Offline Mode"].tap()
    app.buttons["Downloads"].tap()
    XCTAssertTrue(app.buttons["Filter Downloads"].waitForExistence(timeout: 5))
    app.buttons["Filter Downloads"].tap()
    let filter = app.textFields["Filter Downloads"]
    XCTAssertTrue(filter.waitForExistence(timeout: 5))
    filter.tap()
    filter.typeText("1.03")
    XCTAssertTrue(app.staticTexts["A Track 1.03"].firstMatch.waitForExistence(timeout: 5))
    app.staticTexts["A Track 1.03"].firstMatch.tap()
    XCTAssertTrue(app.staticTexts["Details"].waitForExistence(timeout: 5))
    app.swipeUp()
    XCTAssertTrue(app.staticTexts["Unavailable offline"].waitForExistence(timeout: 5))
    capture(app, "Offline Track Details")
    app.swipeRight()
    XCTAssertTrue(app.textFields["Filter Downloads"].waitForExistence(timeout: 5))
    XCTAssertEqual(app.textFields["Filter Downloads"].value as? String, "1.03")
    app.buttons["Close Filter"].tap()
    openNowPlaying(app)
    app.buttons["Command Palette"].tap()
    field.tap()
    field.typeText("Toggle Offline Mode")
    app.buttons["Toggle Offline Mode"].tap()
  }

  func testHomeScreenQuickSearch() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    XCUIDevice.shared.press(.home)
    let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    let icon = springboard.icons["Textamp"].firstMatch
    XCTAssertTrue(icon.waitForExistence(timeout: 5))
    icon.press(forDuration: 1.1)
    let search = springboard.buttons["Search"].firstMatch
    if search.exists { search.tap() } else { springboard.staticTexts["Search"].firstMatch.tap() }
    XCTAssertTrue(app.textFields["Search your library"].waitForExistence(timeout: 10))
    XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
  }

  func testPlaylistSearchOpensTracks() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    app.buttons["bottom-search"].tap()
    let field = app.textFields["Search your library"]
    field.tap()
    field.typeText("Fixture Playlist")
    app.staticTexts["Fixture Playlist"].firstMatch.tap()
    XCTAssertTrue(app.buttons["playlist-edit"].waitForExistence(timeout: 10))
    XCTAssertTrue(app.tables["library-column"].cells.firstMatch.waitForExistence(timeout: 5))
  }

  func testRecentSearchResultReopensPlaylistDirectly() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    app.buttons["bottom-search"].tap()
    let field = app.textFields["Search your library"]
    field.tap()
    field.typeText("Fixture Playlist")
    app.staticTexts["Fixture Playlist"].firstMatch.tap()
    XCTAssertTrue(app.buttons["playlist-edit"].waitForExistence(timeout: 10))
    app.buttons["bottom-search"].tap()
    let previous = app.collectionViews.buttons.containing(.staticText, identifier: "Fixture Playlist").firstMatch
    XCTAssertTrue(previous.waitForExistence(timeout: 5))
    previous.tap()
    XCTAssertTrue(app.buttons["playlist-edit"].waitForExistence(timeout: 10))
    XCTAssertFalse(field.exists)
    app.buttons["bottom-search"].tap()
    field.tap()
    field.typeText("unchosen query")
    app.navigationBars.buttons.element(boundBy: app.navigationBars.buttons.count - 1).tap()
    app.buttons["bottom-search"].tap()
    XCTAssertTrue(previous.waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["unchosen query"].exists)
  }

  func testFuzzySearchShowsResultsWhileTyping() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    if app.buttons["Add Library"].waitForExistence(timeout: 2) { connectFixture(app) }
    dismissIconNotice(XCUIApplication(bundleIdentifier: "com.apple.springboard"), timeout: 2)

    app.buttons["bottom-search"].tap()
    let field = app.textFields["Search your library"]
    XCTAssertTrue(field.waitForExistence(timeout: 5))
    field.tap()
    field.typeText("B")
    let artist = app.staticTexts["B Artist"].firstMatch
    XCTAssertTrue(artist.waitForExistence(timeout: 5))
    field.typeText(" Track 1.03")
    let track = app.staticTexts["B Track 1.03"].firstMatch
    XCTAssertTrue(track.waitForExistence(timeout: 5))
    XCTAssertTrue(app.navigationBars["Search"].exists)
  }

  func testSearchDetailsAndArtworkPreferences() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    connectFixture(app)
    app.buttons["Search"].tap()
    let field = app.textFields["Search your library"]
    XCTAssertTrue(field.waitForExistence(timeout: 5))
    field.tap()
    field.typeText("A Track 1.03\n")
    XCTAssertTrue(app.navigationBars["Search"].exists)
    let result = app.staticTexts["A Track 1.03"].firstMatch
    XCTAssertTrue(result.waitForExistence(timeout: 10))
    result.tap()
    XCTAssertTrue(app.staticTexts["Details"].waitForExistence(timeout: 10))
    XCTAssertTrue(app.staticTexts["A Track 1.03"].firstMatch.exists)
    capture(app, "Search Track Details")
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Textamp"].tap()
    app.buttons["Settings…"].tap()
    app.buttons["Appearance"].tap()
    XCTAssertTrue(app.buttons["system"].waitForExistence(timeout: 5))
    app.buttons["system"].tap()
    let ansi = app.buttons["ANSI half blocks"]
    for _ in 0..<5 {
      if ansi.exists && ansi.isHittable { break }
      app.swipeUp()
    }
    XCTAssertTrue(ansi.exists)
    ansi.tap()
    capture(app, "System Appearance")
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.buttons["Done"].tap()
    openNowPlaying(app)
    capture(app, "ANSI Now Playing")
  }

  func testColumnNavigationAndFiltering() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    checkPalette(app)
    let albums = app.buttons["sidebar-disclosure-Albums"]
    XCTAssertTrue(albums.waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["Random"].exists)
    capture(app, "Library Categories")
    albums.tap()
    XCTAssertTrue(app.buttons["Random"].waitForExistence(timeout: 3))
    capture(app, "Album Disclosure")
    albums.tap()
    XCTAssertFalse(app.buttons["Random"].exists)
    app.buttons["Artists"].tap()
    XCTAssertTrue(app.staticTexts["A Artist"].waitForExistence(timeout: 10))
    capture(app, "Indexed Artists")
    let table = app.tables["library-column"]
    XCTAssertTrue(table.exists)
    XCTAssertFalse(table.otherElements["Section index"].exists)
    app.staticTexts["A Artist"].tap()
    XCTAssertTrue(albumEntry(app, "A Album 1 (2000)").waitForExistence(timeout: 8))
    albumEntry(app, "A Album 1 (2000)").press(forDuration: 0.8)
    XCTAssertTrue(app.buttons["Play Album Next"].waitForExistence(timeout: 3))
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.05, dy: 0.1)).tap()
    albumEntry(app, "A Album 1 (2000)").tap()
    XCTAssertTrue(app.buttons["Play Album"].waitForExistence(timeout: 8))
    XCTAssertTrue(app.buttons["Play Album"].exists)
    capture(app, "Album Tracks")
    app.buttons["column-title"].tap()
    let filter = app.textFields.matching(
      NSPredicate(format: "placeholderValue BEGINSWITH 'Filter '")
    ).firstMatch
    XCTAssertTrue(filter.waitForExistence(timeout: 5))
    filter.typeText("1.03")
    XCTAssertEqual(filter.value as? String, "1.03")
    XCTAssertTrue(app.staticTexts["A Track 1.03"].waitForExistence(timeout: 5))
    wait(
      for: [
        expectation(
          for: NSPredicate { _, _ in !app.staticTexts["A Track 1.04"].exists },
          evaluatedWith: app)
      ], timeout: 5)
    capture(app, "Column Title Filter")
    app.staticTexts["A Track 1.03"].tap()
    XCTAssertTrue(app.buttons["Play Track"].waitForExistence(timeout: 5))
    app.navigationBars.buttons.element(boundBy: 0).tap()
    XCTAssertTrue(filter.waitForExistence(timeout: 5))
    XCTAssertEqual(filter.value as? String, "1.03")
    filter.tap()
    filter.typeText("\n")
    XCTAssertTrue(app.buttons["bottom-search"].isHittable)
    app.buttons["bottom-search"].tap()
    XCTAssertTrue(app.textFields["Search your library"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.segmentedControls.buttons["This Column"].exists)
    app.buttons["Done"].tap()
    app.buttons["Close Filter"].tap()
    XCTAssertTrue(app.staticTexts["A Track 1.04"].waitForExistence(timeout: 5))

  }

  func testPlaybackSheetAndQueue() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    app.buttons["Artists"].tap()
    XCTAssertTrue(app.staticTexts["A Artist"].waitForExistence(timeout: 10))
    app.staticTexts["A Artist"].tap()
    XCTAssertTrue(albumEntry(app, "A Album 1 (2000)").waitForExistence(timeout: 8))
    albumEntry(app, "A Album 1 (2000)").tap()
    XCTAssertTrue(app.buttons["Play Album"].waitForExistence(timeout: 8))
    app.buttons["Play Album"].tap()
    openNowPlaying(app)
    XCTAssertTrue(app.buttons["Close Now Playing"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.buttons["Lyrics"].exists)
    XCTAssertFalse(app.scrollViews.buttons["Play Queue"].exists)
    capture(app, "Now Playing Sheet")
    app.scrollViews.buttons["Next Track"].tap()
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.08)).press(
      forDuration: 0.05,
      thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.8)))
    expectation(
      for: NSPredicate(format: "exists == false"), evaluatedWith: app.buttons["Close Now Playing"])
    waitForExpectations(timeout: 5)
    XCTAssertTrue(app.buttons["bottom-queue"].waitForExistence(timeout: 5))
    app.buttons["bottom-queue"].tap()
    let first = app.staticTexts["A Track 1.01"].firstMatch
    let second = app.staticTexts["A Track 1.02"].firstMatch
    XCTAssertTrue(first.waitForExistence(timeout: 5))
    XCTAssertTrue(second.exists)
    XCTAssertLessThan(first.frame.minY, second.frame.minY)
    app.buttons["Queue Tools"].tap()
    XCTAssertTrue(
      app.buttons.matching(NSPredicate(format: "label CONTAINS[c] 'Save'")).firstMatch
        .waitForExistence(timeout: 5))
    capture(app, "Queue Tools")
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.05, dy: 0.1)).tap()
    checkPalette(app)
  }

  func testFourTapsOnAboutLogoOpenActivityHistory() {
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    runCommand(app, query: "Settings", label: "Settings…")
    XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
    let logo = app.images["about-logo"]
    for _ in 0..<5 where !logo.isHittable { app.swipeUp() }
    XCTAssertTrue(logo.isHittable)
    logo.tap(withNumberOfTaps: 4, numberOfTouches: 1)
    XCTAssertTrue(app.navigationBars["Activity Log"].waitForExistence(timeout: 5))
  }

  func testAboutLogoUsesSelectedTheme() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    dismissIconNotice(springboard, timeout: 2)
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Textamp"].tap()
    app.buttons["Settings…"].tap()
    let logo = app.images["about-logo"]
    let bottomBar = app.buttons["bottom-search"]
    if UIDevice.current.userInterfaceIdiom == .phone {
      XCTAssertTrue(app.buttons["bottom-library"].isHittable)
    }
    func logoImage() -> XCUIScreenshot {
      for _ in 0..<5 {
        if logo.exists && logo.isHittable && logo.frame.maxY <= bottomBar.frame.minY { break }
        app.swipeUp()
      }
      XCTAssertTrue(logo.isHittable)
      XCTAssertLessThanOrEqual(logo.frame.maxY, bottomBar.frame.minY)
      return logo.screenshot()
    }
    let before = logoImage()
    for _ in 0..<5 {
      if app.buttons["Appearance"].isHittable { break }
      app.swipeDown()
    }
    app.buttons["Appearance"].tap()
    let picker = app.buttons["preference-choice-Theme"]
    let current = picker.value as? String
    picker.tap()
    app.buttons[current?.lowercased() == "norton" ? "system" : "norton"].tap()
    dismissIconNotice(springboard, timeout: 3)
    app.navigationBars.buttons.element(boundBy: 0).tap()
    let after = logoImage()
    XCTAssertNotEqual(before.pngRepresentation, after.pngRepresentation)
    for (name, screenshot) in [("Original About logo", before), ("Recolored About logo", after)] {
      let attachment = XCTAttachment(screenshot: screenshot)
      attachment.name = name
      attachment.lifetime = .keepAlways
      add(attachment)
    }
  }

  func testThemeChangesTheApplicationIcon() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    dismissIconNotice(springboard, timeout: 5)
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Textamp"].tap()
    app.buttons["Settings…"].tap()
    app.buttons["Appearance"].tap()
    let system = app.buttons["system"]
    system.tap()
    dismissIconNotice(springboard, timeout: 5)
    let norton = app.buttons["norton"]
    for _ in 0..<5 {
      if norton.exists && norton.isHittable { break }
      app.swipeUp()
    }
    norton.tap()
    wait(
      for: [expectation(for: NSPredicate(format: "selected == true"), evaluatedWith: norton)],
      timeout: 5)
    dismissIconNotice(springboard, timeout: 5)
    capture(app, "Norton Icon Theme")
    app.activate()
    system.tap()
    dismissIconNotice(springboard, timeout: 5)
    capture(app, "System Icon Theme")
  }

  private func dismissIconNotice(_ springboard: XCUIApplication, timeout: TimeInterval) {
    let notice = springboard.alerts.firstMatch
    if notice.waitForExistence(timeout: timeout),
      notice.staticTexts.matching(NSPredicate(format: "label CONTAINS[c] 'icon'")).count > 0
    {
      notice.buttons["OK"].tap()
    }
  }

  func testSlowUpwardDragOpensNowPlayingOnRelease() {
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    let transport = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Now Playing, '")).firstMatch
    XCTAssertTrue(transport.waitForExistence(timeout: 10))
    let start = transport.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
    start.press(forDuration: 0.1, thenDragTo: start.withOffset(CGVector(dx: 0, dy: -160)),
      withVelocity: .slow, thenHoldForDuration: 1)
    let close = app.buttons["Close Now Playing"]
    XCTAssertTrue(close.waitForExistence(timeout: 5))
    XCTAssertLessThan(close.frame.midY, app.frame.height * 0.25)
    close.tap()
    XCTAssertTrue(transport.waitForExistence(timeout: 5))
  }

  func testNowPlayingCenterSwipeDismissesToTransport() {
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    openNowPlaying(app)
    let close = app.buttons["Close Now Playing"]
    XCTAssertTrue(close.waitForExistence(timeout: 5))
    let center = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
    center.press(forDuration: 0.05, thenDragTo:
      app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.91)),
      withVelocity: .fast, thenHoldForDuration: 0)
    let dismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: close)
    XCTAssertEqual(XCTWaiter.wait(for: [dismissed], timeout: 5), .completed)
    XCTAssertTrue(app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Now Playing, '")).firstMatch.isHittable)
  }

  func testModalNavigationUsesOnlyTheCustomBottomBar() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    let transport = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Now Playing, '")).firstMatch
    XCTAssertTrue(transport.waitForExistence(timeout: 10))
    let transportFrame = transport.frame
    XCTAssertEqual(app.tabBars.count, 0)
    openNowPlaying(app)
    XCTAssertFalse(app.buttons["Expand Artwork"].exists)
    capture(app, "Inline Now Playing Artwork")
    app.buttons["Close Now Playing"].tap()
    XCTAssertTrue(transport.waitForExistence(timeout: 5))
    XCTAssertEqual(transport.frame, transportFrame)
    openNowPlaying(app)
    let close = app.buttons["Close Now Playing"]
    XCTAssertTrue(close.waitForExistence(timeout: 5))
    XCTAssertGreaterThan(close.frame.midX, app.frame.midX)
    XCTAssertLessThan(close.frame.midY, app.frame.height * 0.25)
    let grabber = app.buttons["Minimize Now Playing"]
    XCTAssertTrue(grabber.exists)
    let title = app.navigationBars["Now Playing"].staticTexts["Now Playing"]
    let top = title.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
    top.press(forDuration: 0.1, thenDragTo: top.withOffset(CGVector(dx: 0, dy: 80)),
      withVelocity: .slow, thenHoldForDuration: 0.4)
    XCTAssertLessThan(close.frame.midY, app.frame.height * 0.25)
    top.press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.55)),
      withVelocity: .slow, thenHoldForDuration: 0.4)
    XCTAssertTrue(transport.waitForExistence(timeout: 5))
    XCTAssertEqual(transport.frame, transportFrame)
    openNowPlaying(app)
    XCTAssertTrue(close.waitForExistence(timeout: 5))
    title.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).press(
      forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.94)),
      withVelocity: .fast, thenHoldForDuration: 0)
    XCTAssertTrue(transport.waitForExistence(timeout: 5), "A full swipe should dismiss in one gesture")
    openNowPlaying(app)
    XCTAssertTrue(grabber.waitForExistence(timeout: 5))
    grabber.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).press(
      forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.94)),
      withVelocity: .fast, thenHoldForDuration: 0)
    XCTAssertTrue(transport.waitForExistence(timeout: 5), "A full swipe from the handle should dismiss")
    for _ in 0..<2 {
      app.buttons["bottom-search"].tap()
      XCTAssertTrue(app.textFields["Search your library"].waitForExistence(timeout: 5))
      XCTAssertEqual(app.tabBars.count, 0)
      capture(app, "Search Without Native Tab Bar")
      XCTAssertGreaterThan(app.buttons["Done"].frame.midX, app.frame.midX)
      app.buttons["Done"].tap()
      XCTAssertTrue(app.buttons["Command Palette"].waitForExistence(timeout: 5))
      app.buttons["Command Palette"].tap()
      XCTAssertTrue(app.textFields["Find a command"].waitForExistence(timeout: 5))
      XCTAssertEqual(app.tabBars.count, 0)
      app.buttons["Done"].tap()
    }
    XCTAssertTrue(app.buttons["bottom-library"].isHittable)
    XCTAssertTrue(app.buttons["bottom-queue"].isHittable)
    app.buttons["bottom-queue"].tap()
    XCTAssertTrue(app.navigationBars["Play Queue"].buttons["Edit Queue"].waitForExistence(timeout: 5))
    app.terminate()
  }

  func testNativePlaybackSeekingKeepsPlayingAndPausedStates() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    openNowPlaying(app)
    let volume = app.sliders["Volume"]
    if !volume.isHittable { app.swipeUp() }
    XCTAssertTrue(volume.isHittable)
    volume.adjust(toNormalizedSliderPosition: 0)
    app.buttons["Close Now Playing"].tap()
    app.buttons["bottom-library"].tap()
    app.buttons["Artists"].tap()
    app.staticTexts["A Artist"].tap()
    albumEntry(app, "A Album 1 (2000)").tap()
    app.cells.matching(NSPredicate(format: "identifier ENDSWITH 'song-0-0-0'")).firstMatch.tap()
    app.buttons["Play Track"].tap()
    openNowPlaying(app)
    let toggle = app.buttons["now-playing-toggle"]
    XCTAssertTrue(toggle.waitForExistence(timeout: 8))
    let position = app.sliders["waveform-position"]
    let waveform = app.descendants(matching: .any).matching(identifier: "playback-waveform")
      .firstMatch
    XCTAssertTrue(position.waitForExistence(timeout: 8))
    XCTAssertTrue(
      waveform.frame.intersects(position.frame), "The scrubber must overlay the waveform")
    position.adjust(toNormalizedSliderPosition: 0.5)
    wait(
      for: [
        expectation(
          for: NSPredicate { _, _ in
            let value = waveform.value as? String ?? ""
            return value.hasPrefix("0:3")
          }, evaluatedWith: app)
      ], timeout: 8)
    XCTAssertEqual(toggle.label, "Pause")
    toggle.tap()
    waveform.coordinate(withNormalizedOffset: CGVector(dx: 0.75, dy: 0.5)).tap()
    wait(
      for: [
        expectation(
          for: NSPredicate { _, _ in
            ["0:44", "0:45", "0:46"].contains(waveform.value as? String ?? "")
          }, evaluatedWith: app)
      ], timeout: 8)
    XCTAssertEqual(toggle.label, "Play")
    let pausedTime = waveform.value as? String ?? ""
    XCTAssertEqual(
      app.staticTexts.matching(identifier: pausedTime).count, 1,
      "Now Playing must expose one authoritative transport clock")
    capture(app, "Paused After Native Seek")
    let pausedPosition = waveform.value as? String
    toggle.tap()
    wait(
      for: [
        expectation(
          for: NSPredicate { _, _ in waveform.value as? String != pausedPosition },
          evaluatedWith: app)
      ], timeout: 5)
    XCTAssertEqual(toggle.label, "Pause")
    app.terminate()
  }

  func testFollowingLyricsCentersAndEnlargesTheCurrentLine() throws {
    continueAfterFailure = false
    let app = openAlbumDetails("B Album 1 (2001)", artist: "B Artist", silent: false)
    openNowPlaying(app)
    app.sliders["Volume"].adjust(toNormalizedSliderPosition: 0)
    app.buttons["Close Now Playing"].tap()
    XCTAssertTrue(app.buttons["Close Now Playing"].waitForNonExistence(timeout: 5))
    app.buttons["Play Album"].tap()
    openNowPlaying(app)
    let position = app.sliders["waveform-position"]
    let waveform = app.descendants(matching: .any)["playback-waveform"]
    let advancing = expectation(for: NSPredicate { _, _ in
      let value = waveform.value as? String ?? ""
      return value.hasPrefix("0:") && value != "0:00"
    }, evaluatedWith: app)
    wait(for: [advancing], timeout: 12)
    position.adjust(toNormalizedSliderPosition: 0.2)
    app.buttons["Lyrics"].tap()
    let active = app.staticTexts["lyrics-active-line"]
    XCTAssertTrue(active.waitForExistence(timeout: 8), app.debugDescription)
    let follow = app.switches["lyrics-follow"]
    let scroller = app.scrollViews["lyrics-scroll"]
    func centered() -> Bool {
      guard active.exists, scroller.exists else { return false }
      let viewport = scroller.frame
      return active.frame.midY > viewport.minY + viewport.height / 3
        && active.frame.midY < viewport.maxY - viewport.height / 3
    }
    let centeredInitially = expectation(for: NSPredicate { _, _ in centered() }, evaluatedWith: app)
    XCTAssertEqual(XCTWaiter.wait(for: [centeredInitially], timeout: 5), .completed,
                   "Active \(active.label) \(active.frame), scroll view \(scroller.frame)")
    let firstLine = active.label
    let nextLine = expectation(for: NSPredicate(format: "label != %@", firstLine), evaluatedWith: active)
    wait(for: [nextLine], timeout: 12)
    let number = try XCTUnwrap(Int(active.label.split(separator: " ").last ?? ""))
    let adjacent = app.staticTexts["lyrics-line-\(number)"]
    XCTAssertGreaterThan(active.frame.height, adjacent.frame.height)
    follow.switches.firstMatch.tap()
    XCTAssertTrue(active.waitForNonExistence(timeout: 5))
    scroller.swipeUp()
    follow.switches.firstMatch.tap()
    let recentered = expectation(for: NSPredicate { _, _ in centered() }, evaluatedWith: app)
    wait(for: [recentered], timeout: 5)
  }

  func testLyricsFollowsThePlaybackClock() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--test-profile=fixture-validation"]
    app.launch()
    if !app.buttons["Artists"].waitForExistence(timeout: 5) { connectFixture(app) }
    openNowPlaying(app)
    app.sliders["Volume"].adjust(toNormalizedSliderPosition: 0)
    app.buttons["Close Now Playing"].tap()
    app.buttons["Artists"].tap()
    app.staticTexts["A Artist"].tap()
    albumEntry(app, "A Album 1 (2000)").tap()
    app.cells.matching(NSPredicate(format: "identifier ENDSWITH 'song-0-0-0'")).firstMatch.tap()
    app.buttons["Play Track"].tap()
    openNowPlaying(app)
    app.sliders["waveform-position"].adjust(toNormalizedSliderPosition: 0.32)
    app.buttons["Lyrics"].tap()
    let current = app.staticTexts["lyrics-active-line"]
    XCTAssertTrue(current.waitForExistence(timeout: 5))
    XCTAssertEqual(current.label, "Synthetic fixture lyrics.")
    XCTAssertFalse(app.toolbars.buttons["Command Palette"].exists)
    let advance = expectation(for: NSPredicate(format: "label == %@", "Second fixture line."),
                              evaluatedWith: current)
    wait(for: [advance], timeout: 16)
    app.buttons["lyrics-language"].tap()
    app.buttons["fr · 2"].tap()
    XCTAssertFalse(app.switches["lyrics-follow"].isEnabled)
    let lyricsClose = app.buttons["Done"]
    XCTAssertLessThan(lyricsClose.frame.midY, app.frame.height * 0.25)
    app.buttons["Sheet Grabber"].tap()
    XCTAssertGreaterThan(lyricsClose.frame.midY, app.frame.height * 0.4)
    app.buttons["Sheet Grabber"].tap()
    XCTAssertLessThan(lyricsClose.frame.midY, app.frame.height * 0.25)
    let header = app.navigationBars["Lyrics"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
    header.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.94)),
                 withVelocity: .fast, thenHoldForDuration: 0)
    XCTAssertTrue(app.buttons["Close Now Playing"].waitForExistence(timeout: 5))
    app.buttons["Command Palette"].tap()
    let album = app.buttons["Open in Library"]
    XCTAssertTrue(album.waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["palette-group:Contextual Actions"].exists)
    let view = app.buttons["palette-group:View"]
    let list = app.collectionViews.firstMatch
    for _ in 0..<12 where !view.isHittable { list.swipeUp() }
    view.tap()
    app.buttons["palette-group:View/Browse"].tap()
    XCTAssertTrue(album.waitForExistence(timeout: 5))
    app.terminate()
  }

  func testDescribeResultsPlaybackAndSaveControl() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launch()
    connectFixture(app)
    openNowPlaying(app)
    let volume = app.sliders["Volume"]
    if !volume.isHittable { app.swipeUp() }
    volume.adjust(toNormalizedSliderPosition: 0)
    app.buttons["Close Now Playing"].tap()
    app.buttons["bottom-library"].tap()
    if !app.buttons["Describe Music"].exists { app.buttons["sidebar-disclosure-AI"].tap() }
    for _ in 0..<3 where !app.buttons["Describe Music"].exists { app.swipeUp() }
    XCTAssertTrue(app.buttons["Describe Music"].waitForExistence(timeout: 5))
    app.buttons["Describe Music"].tap()
    let history = app.staticTexts["Fixture discoveries"]
    XCTAssertTrue(history.waitForExistence(timeout: 8))
    history.tap()
    let play = app.buttons["Play All Tracks"]
    let save = app.buttons["Save as Playlist"]
    XCTAssertTrue(play.waitForExistence(timeout: 8))
    XCTAssertTrue(save.exists)
    XCTAssertFalse(save.staticTexts.firstMatch.exists)
    capture(app, "Describe Music Result Actions")
    play.tap()
    openNowPlaying(app)
    let toggle = app.buttons["now-playing-toggle"]
    wait(
      for: [expectation(for: NSPredicate { _, _ in toggle.label == "Pause" }, evaluatedWith: app)],
      timeout: 8)
    let waveform = app.descendants(matching: .any).matching(identifier: "playback-waveform")
      .firstMatch
    wait(
      for: [
        expectation(
          for: NSPredicate { _, _ in
            let value = waveform.value as? String ?? ""
            return value.hasPrefix("0:") && value != "0:00"
          }, evaluatedWith: app)
      ], timeout: 8)
    toggle.tap()
    XCTAssertEqual(toggle.label, "Play")
    capture(app, "Describe Music Result Playback")
    app.terminate()
  }

  func testLibraryErrorStaysOutsideCommandPalette() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-failure"]
    app.launch()
    allowFixtureNetwork()
    app.activate()
    capture(app, "Failure fixture initial screen")
    let retry = app.buttons["retry-library"]
    XCTAssertTrue(retry.waitForExistence(timeout: 10))
    let statusFrame = retry.frame
    openNowPlaying(app)
    let close = app.buttons["Close Now Playing"]
    XCTAssertTrue(close.waitForExistence(timeout: 5))
    XCTAssertEqual(app.buttons.matching(identifier: "retry-library").count, 1)
    XCTAssertEqual(retry.frame, statusFrame)
    close.tap()
    XCTAssertTrue(retry.isHittable)
    XCTAssertEqual(retry.frame, statusFrame)
    app.buttons["Command Palette"].firstMatch.tap()
    XCTAssertTrue(app.textFields["Find a command"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["retry-library"].exists)
    capture(app, "Palette without library error overlay")
    app.navigationBars["Command Palette"].buttons["Done"].tap()
    XCTAssertTrue(app.buttons["retry-library"].waitForExistence(timeout: 5))
    capture(app, "Library error returns on main surface")
  }

  func testDeviceLibraryLoadingAndNavigation() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=live-performance"]
    app.launchEnvironment["RUST_LOG"] =
      "error,textamp::mobile=info,textamp::app::sources::navidrome=info,textamp::navidrome=warn"
    let started = ProcessInfo.processInfo.systemUptime
    app.launch()
    XCTAssertTrue(app.buttons["Artists"].waitForExistence(timeout: 60))
    print("DEVICE_LIBRARY_UI_READY_SECONDS: \(ProcessInfo.processInfo.systemUptime - started)")
    capture(app, "Device cached library root")
    app.buttons["Artists"].tap()
    let column = app.tables.matching(identifier: "library-column").firstMatch
    XCTAssertTrue(column.waitForExistence(timeout: 20))
    column.swipeUp()
    column.swipeUp()
    column.swipeDown()
    capture(app, "Device artist scrolling")
    runCommand(app, query: "Settings", label: "Settings…")
    XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))
    app.navigationBars["Settings"].buttons["Done"].tap()
    app.buttons["bottom-queue"].tap()
    XCTAssertTrue(app.navigationBars["Play Queue"].waitForExistence(timeout: 10))
    app.buttons["bottom-library"].tap()
    runCommand(app, query: "Refresh", label: "Refresh")
    capture(app, "Device refresh remains interactive")
    runCommand(app, query: "Settings", label: "Settings…")
    XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))
    app.navigationBars["Settings"].buttons["Done"].tap()
    capture(app, "Device navigation completed")
  }

  func testDevicePlaylistReopensFromPersistentCache() throws {
    let environment = ProcessInfo.processInfo.environment
    guard let playlist = environment["TEXTAMP_DEVICE_PLAYLIST"],
      let track = environment["TEXTAMP_DEVICE_TRACK"]
    else { throw XCTSkip("A read-only device library scenario must be provided") }
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=live-playlist"]
    for pass in 0..<2 {
      app.launch()
      let playlists = app.buttons["sidebar-open-Playlists"]
      XCTAssertTrue(playlists.waitForExistence(timeout: 30))
      playlists.tap()
      let row = app.tables.staticTexts[playlist].firstMatch
      XCTAssertTrue(row.waitForExistence(timeout: 15))
      row.tap()
      XCTAssertTrue(app.tables.staticTexts[track].firstMatch.waitForExistence(timeout: 20))
      capture(app, pass == 0 ? "Device playlist first opening" : "Device playlist cached reopening")
      app.terminate()
    }
  }

  func testFixtureNavigationPerformance() throws {
    guard #available(iOS 26.0, *) else { throw XCTSkip("Hitch metrics require iOS 26") }
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    let options = XCTMeasureOptions()
    options.iterationCount = 3
    measure(
      metrics: [
        XCTCPUMetric(application: app), XCTMemoryMetric(application: app),
        XCTHitchMetric(application: app),
      ], options: options
    ) {
      app.buttons["Artists"].tap()
      XCTAssertTrue(app.staticTexts["A Artist"].firstMatch.waitForExistence(timeout: 5))
      runCommand(app, query: "Settings", label: "Settings…")
      XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
      app.navigationBars["Settings"].buttons["Done"].tap()
      app.buttons["bottom-library"].tap()
      XCTAssertTrue(app.buttons["Artists"].waitForExistence(timeout: 5))
    }
  }

  func testInlineThemeAndVisibleVolumeWithLyricsFollowAndLanguage() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=lyrics-validation"]
    app.launch()
    connectFixture(app)
    runCommand(app, query: "Settings", label: "Settings…")
    XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
    app.buttons["Appearance"].tap()
    XCTAssertTrue(app.navigationBars["Appearance"].waitForExistence(timeout: 5))
    let theme = app.buttons["preference-choice-Theme"]
    XCTAssertTrue(theme.waitForExistence(timeout: 5))
    theme.tap()
    app.buttons["solarized dark"].tap()
    dismissIconNotice(XCUIApplication(bundleIdentifier: "com.apple.springboard"), timeout: 1)
    XCTAssertTrue(app.navigationBars["Appearance"].exists)
    XCTAssertTrue(theme.isHittable)
    capture(app, "Inline Solarized Dark theme")
    theme.tap()
    app.buttons["system"].tap()
    dismissIconNotice(XCUIApplication(bundleIdentifier: "com.apple.springboard"), timeout: 1)
    XCTAssertTrue(app.navigationBars["Appearance"].exists)
    returnToSettingsPage(app, title: "Settings")
    app.buttons["Done"].tap()
    openNowPlaying(app)
    XCTAssertTrue(app.sliders["Volume"].isHittable, "Volume must be visible without scrolling")
    XCUIDevice.shared.orientation = .landscapeLeft
    XCTAssertTrue(app.sliders["Volume"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.sliders["Volume"].isHittable)
    XCUIDevice.shared.orientation = .portrait
    app.buttons["Close Now Playing"].tap()
    app.buttons["bottom-library"].tap()
    app.buttons["Artists"].tap()
    app.staticTexts["A Artist"].tap()
    albumEntry(app, "A Album 1 (2000)").tap()
    app.cells.matching(NSPredicate(format: "identifier ENDSWITH 'song-0-0-0'")).firstMatch.tap()
    app.buttons["Play Track"].tap()
    openNowPlaying(app)
    XCTAssertTrue(app.sliders["Volume"].isHittable)
    app.buttons["Lyrics"].tap()
    let follow = app.switches["lyrics-follow"]
    XCTAssertTrue(follow.waitForExistence(timeout: 8))
    XCTAssertTrue(app.staticTexts["Synthetic fixture lyrics."].waitForExistence(timeout: 8))
    XCTAssertFalse(app.toolbars.buttons["Command Palette"].exists)
    follow.switches.firstMatch.tap()
    XCTAssertEqual(follow.value as? String, "0")
    XCTAssertFalse(app.staticTexts["lyrics-active-line"].exists)
    follow.switches.firstMatch.tap()
    XCTAssertEqual(follow.value as? String, "1")
    app.buttons["lyrics-language"].tap()
    app.buttons["fr · 2"].tap()
    XCTAssertTrue(app.staticTexts["Paroles de test."].waitForExistence(timeout: 5))
    XCTAssertFalse(follow.isEnabled)
    capture(app, "Plain lyrics language")
    app.buttons["Done"].tap()
    XCTAssertTrue(app.sliders["Volume"].isHittable)
    app.terminate()
  }

  func testSettingsUseNativeSwitchesPickersAndSteppers() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    runCommand(app, query: "Settings", label: "Settings…")
    XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
    app.buttons["Appearance"].tap()
    XCTAssertTrue(
      app.navigationBars["Appearance"].waitForExistence(timeout: 5), app.debugDescription)
    let theme = app.buttons["preference-choice-Theme"]
    XCTAssertTrue(theme.waitForExistence(timeout: 5))
    theme.tap()
    XCTAssertTrue(app.buttons["system"].waitForExistence(timeout: 5))
    capture(app, "Native theme picker")
    app.buttons["system"].tap()
    returnToSettingsPage(app, title: "Appearance")
    let artwork = app.buttons["artwork-mode"]
    XCTAssertTrue(artwork.waitForExistence(timeout: 5))
    artwork.tap()
    XCTAssertTrue(app.buttons["Images"].waitForExistence(timeout: 5))
    app.buttons["Images"].tap()
    returnToSettingsPage(app, title: "Appearance")
    returnToSettingsPage(app, title: "Settings")
    app.buttons["Behavior"].tap()
    let youtube = app.switches["preference-YouTube"]
    XCTAssertTrue(youtube.waitForExistence(timeout: 5))
    let original = youtube.value as? String
    youtube.switches.firstMatch.tap()
    wait(
      for: [
        expectation(
          for: NSPredicate { _, _ in youtube.value as? String != original },
          evaluatedWith: youtube)
      ], timeout: 5)
    youtube.switches.firstMatch.tap()
    wait(
      for: [
        expectation(
          for: NSPredicate { _, _ in youtube.value as? String == original },
          evaluatedWith: youtube)
      ], timeout: 5)
    let quality = app.buttons["preference-choice-Streaming quality"]
    if !quality.isHittable { app.swipeUp() }
    quality.tap()
    app.buttons["Original audio"].tap()
    returnToSettingsPage(app, title: "Behavior")
    quality.tap()
    app.buttons["Up to 192 kbps"].tap()
    returnToSettingsPage(app, title: "Behavior")
    XCTAssertTrue(quality.waitForExistence(timeout: 5))
    XCTAssertTrue(
      quality.label.contains("192") || (quality.value as? String)?.contains("192") == true)
    quality.tap()
    app.buttons["Original audio"].tap()
    returnToSettingsPage(app, title: "Behavior")
    capture(app, "Native preference switches and streaming picker")
    returnToSettingsPage(app, title: "Settings")
    app.buttons["settings-section-Library"].tap()
    XCTAssertTrue(app.steppers.firstMatch.waitForExistence(timeout: 5))
    XCTAssertTrue(app.switches.firstMatch.exists)
    capture(app, "Native library switches and count stepper")
  }

  private func returnToSettingsPage(_ app: XCUIApplication, title: String) {
    if app.navigationBars[title].waitForExistence(timeout: 2) { return }
    app.navigationBars.buttons.element(boundBy: 0).tap()
    XCTAssertTrue(app.navigationBars[title].waitForExistence(timeout: 5))
  }

  func testSwipeDismissesSearchKeyboardAndKeepsQuery() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=keyboard-swipe"]
    app.launch()
    app.buttons["bottom-search"].tap()
    let query = app.textFields["Search your library"]
    XCTAssertTrue(query.waitForExistence(timeout: 5))
    query.tap()
    query.typeText("unmatched")
    swipeBelowField(query)
    XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5))
    XCTAssertEqual(query.value as? String, "unmatched")
    query.tap()
    XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
  }

  func testSwipeDismissesSettingsKeyboardAndKeepsDraft() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=keyboard-swipe"]
    app.launch()
    app.buttons["Add Library"].tap()
    app.buttons["Add Navidrome Library"].tap()
    let field = app.textFields["Server URL"]
    field.tap()
    field.typeText("music.example")
    swipeBelowField(field)
    XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5))
    XCTAssertEqual(field.value as? String, "music.example")
  }

  private func swipeBelowField(_ field: XCUIElement) {
    let start = field.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 1))
      .withOffset(CGVector(dx: 0, dy: 12))
    start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: 75)))
  }

  func testColumnFilterKeyboardDismissesWithoutClearingShortOrEmptyResults() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    app.buttons["Artists"].tap()
    XCTAssertTrue(app.staticTexts["A Artist"].waitForExistence(timeout: 5))
    app.staticTexts["A Artist"].tap()
    XCTAssertTrue(albumEntry(app, "A Album 1 (2000)").waitForExistence(timeout: 5))
    albumEntry(app, "A Album 1 (2000)").tap()
    XCTAssertTrue(app.buttons["column-title"].waitForExistence(timeout: 5))
    app.buttons["column-title"].tap()
    let filter = app.textFields.matching(
      NSPredicate(format: "placeholderValue BEGINSWITH 'Filter '")
    ).firstMatch
    XCTAssertTrue(filter.waitForExistence(timeout: 5))
    filter.typeText("1.03")
    XCTAssertTrue(app.staticTexts["A Track 1.03"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.keyboards.firstMatch.exists)
    app.tables["library-column"].swipeDown(velocity: .slow)
    XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5))
    XCTAssertEqual(filter.value as? String, "1.03")
    XCTAssertTrue(app.staticTexts["A Track 1.03"].exists)
    filter.tap()
    XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
    filter.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 4) + "zzzzz")
    XCTAssertTrue(app.staticTexts["No matches"].waitForExistence(timeout: 5))
    app.tables["library-column"].swipeDown(velocity: .slow)
    XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5))
    XCTAssertEqual(filter.value as? String, "zzzzz")
    capture(app, "Column filter retained after keyboard dismissal")
  }

  func testEveryThemeAcrossNativeNavigationAndSettings() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    let names = [
      "system", "solarized dark", "solarized light", "dark", "dracula", "northern", "hot dog stand",
      "norton", "platinum", "brushed aluminum", "black and white", "white on black", "amber crt", "phosphor green",
    ]
    for name in names {
      app.buttons["Command Palette"].firstMatch.tap()
      XCTAssertTrue(app.buttons["palette-group:Textamp"].waitForExistence(timeout: 5))
      app.buttons["palette-group:Textamp"].tap()
      app.buttons["Settings…"].tap()
      XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
      app.buttons["settings-section-Appearance"].tap()
      XCTAssertTrue(app.buttons["preference-choice-Theme"].waitForExistence(timeout: 5))
      app.buttons["preference-choice-Theme"].tap()
      XCTAssertTrue(app.buttons[name].waitForExistence(timeout: 5))
      app.buttons[name].tap()
      dismissIconNotice(XCUIApplication(bundleIdentifier: "com.apple.springboard"), timeout: 1)
      returnToSettingsPage(app, title: "Appearance")
      capture(app, "Theme \(name) Settings")
      app.buttons["bottom-library"].tap()
      XCTAssertTrue(app.staticTexts["library-heading"].waitForExistence(timeout: 5))
      capture(app, "Theme \(name) Library")
    }
  }

  func testCachedColdLaunchPublishesCompleteLibraryRoot() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=fixture-validation"]
    app.launch()
    connectFixture(app)
    XCTAssertTrue(app.buttons["sidebar-open-Playlists"].waitForExistence(timeout: 8))
    app.terminate()
    app.launch()
    XCTAssertFalse(app.buttons["Add Library"].exists)
    XCTAssertTrue(app.buttons["Artists"].waitForExistence(timeout: 8))
    XCTAssertTrue(app.buttons["sidebar-open-Albums"].exists)
    XCTAssertTrue(app.buttons["sidebar-open-Playlists"].exists)
    capture(app, "Cached Library Root")
  }

  func testPlaylistArtworkSwitchAndPlaybackWaveform() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    connectFixture(app)
    app.buttons["bottom-library"].tap()
    app.buttons["sidebar-open-Playlists"].tap()
    let second = app.cells.matching(NSPredicate(format: "identifier ENDSWITH 'playlist-2'"))
      .firstMatch
    XCTAssertTrue(second.waitForExistence(timeout: 8))
    second.tap()
    let secondTrack = app.cells.matching(NSPredicate(format: "identifier ENDSWITH 'song-1-0-0'"))
      .firstMatch
    XCTAssertTrue(secondTrack.waitForExistence(timeout: 8))
    app.navigationBars.buttons["Back"].tap()
    let first = app.cells.matching(NSPredicate(format: "identifier ENDSWITH 'playlist-1'"))
      .firstMatch
    XCTAssertTrue(first.waitForExistence(timeout: 5))
    first.tap()
    let track = app.cells.matching(NSPredicate(format: "identifier ENDSWITH 'song-0-0-0'"))
      .firstMatch
    XCTAssertTrue(track.waitForExistence(timeout: 8))
    track.tap()
    XCTAssertTrue(app.staticTexts["Details"].waitForExistence(timeout: 5))
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Textamp"].tap()
    app.buttons["Settings…"].tap()
    app.buttons["Appearance"].tap()
    let artwork = app.buttons["artwork-mode"]
    artwork.tap()
    app.buttons["Braille"].tap()
    artwork.tap()
    app.buttons["Images"].tap()
    app.navigationBars["Appearance"].buttons.element(boundBy: 0).tap()
    app.buttons["Done"].tap()
    XCTAssertTrue(app.staticTexts["Details"].waitForExistence(timeout: 5))
    let content = app.scrollViews["detail-content"]
    XCTAssertGreaterThan(content.frame.width, app.frame.width * 0.9)
    XCTAssertTrue(app.staticTexts["A Track 1.01"].firstMatch.exists)
    capture(app, "Track Details After Artwork Mode Change")
    app.buttons["Play Track"].tap()
    openNowPlaying(app)
    let waveform = app.descendants(matching: .any).matching(identifier: "playback-waveform")
      .firstMatch
    XCTAssertTrue(waveform.waitForExistence(timeout: 8))
    XCTAssertTrue(waveform.isHittable)
    let waveformFrame = waveform.frame
    let transportFrame = app.buttons["now-playing-toggle"].frame
    wait(
      for: [
        expectation(
          for: NSPredicate { _, _ in
            let value = waveform.value as? String ?? ""
            return !value.isEmpty && !value.contains("Preparing") && !value.contains("unavailable")
          }, evaluatedWith: waveform)
      ], timeout: 20)
    XCTAssertEqual(waveform.frame.minY, waveformFrame.minY, accuracy: 1)
    XCTAssertEqual(app.buttons["now-playing-toggle"].frame.minY, transportFrame.minY, accuracy: 1)
    let position = app.sliders["Waveform Position"]
    XCTAssertTrue(position.isHittable)
    XCTAssertLessThan(waveform.frame.maxY, app.buttons["bottom-queue"].frame.minY)
    XCTAssertFalse(app.buttons["Expand Waveform"].exists)
    capture(app, "Navigable Playback Waveform")
  }

  func testArtworkDropdownAndThemeChoicesStayInAppearance() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent", "--test-profile=appearance-settings-validation"]
    app.launch()
    connectFixture(app)
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Textamp"].tap()
    app.buttons["Settings…"].tap()
    app.buttons["Appearance"].tap()
    let artwork = app.buttons["artwork-mode"]
    XCTAssertTrue(artwork.waitForExistence(timeout: 5))
    XCTAssertTrue(artwork.isHittable)
    artwork.tap()
    XCTAssertTrue(app.buttons["Braille"].waitForExistence(timeout: 3))
    app.buttons["Braille"].tap()
    XCTAssertTrue(app.navigationBars["Appearance"].exists)
    XCTAssertTrue(artwork.isHittable)
    artwork.tap()
    app.buttons["Images"].tap()
    XCTAssertTrue(app.navigationBars["Appearance"].exists)
    let theme = app.buttons["preference-choice-Theme"]
    theme.tap()
    XCTAssertTrue(app.buttons["northern"].exists)
    app.buttons["hot dog stand"].tap()
    dismissIconNotice(XCUIApplication(bundleIdentifier: "com.apple.springboard"), timeout: 2)
    XCTAssertTrue(app.navigationBars["Appearance"].exists)
    XCTAssertTrue(theme.isHittable)
    capture(app, "Hot Dog Stand Appearance")
    theme.tap()
    app.buttons["system"].tap()
    dismissIconNotice(XCUIApplication(bundleIdentifier: "com.apple.springboard"), timeout: 2)
    capture(app, "Accessible Artwork Settings")
  }

  func testArtworkAndBehaviorSettingsLayout() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Textamp"].tap()
    app.buttons["Settings…"].tap()
    app.buttons["Appearance"].tap()
    let artwork = app.buttons["artwork-mode"]
    XCTAssertTrue(artwork.waitForExistence(timeout: 5))
    XCTAssertTrue(artwork.isHittable)
    artwork.tap()
    XCTAssertTrue(app.buttons["Braille"].waitForExistence(timeout: 3))
    app.buttons["Braille"].tap()
    artwork.tap()
    app.buttons["Images"].tap()
    capture(app, "Accessible Artwork Settings")
    app.navigationBars["Appearance"].buttons.element(boundBy: 0).tap()
    XCTAssertFalse(app.cells.buttons["Search"].exists)
    XCTAssertFalse(app.cells.buttons["Audio"].exists)
    app.buttons["Behavior"].tap()
    XCTAssertTrue(app.staticTexts["Audio"].exists)
    XCTAssertTrue(app.staticTexts["Search"].exists)
    capture(app, "Behavior Settings")
  }

  func testSettingsPaletteAndRadio() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = ["--silent"]
    app.launch()
    connectFixture(app)
    app.buttons["Command Palette"].firstMatch.tap()
    app.buttons["palette-group:Textamp"].tap()
    app.buttons["Settings…"].tap()
    app.buttons["Appearance"].tap()
    checkPalette(app)
    XCTAssertTrue(app.navigationBars["Appearance"].waitForExistence(timeout: 5))
    app.buttons["system"].tap()
    capture(app, "System Settings")
    app.navigationBars["Appearance"].buttons.element(boundBy: 0).tap()
    app.navigationBars["Settings"].buttons["Done"].tap()
    for _ in 0..<6 {
      if app.staticTexts["library-heading"].exists { break }
      app.navigationBars.buttons.element(boundBy: 0).tap()
    }
    capture(app, "System Library")
    for _ in 0..<3 {
      if app.buttons["Radio"].exists && app.buttons["Radio"].isHittable { break }
      app.swipeUp()
    }
    app.buttons["Radio"].tap()
    XCTAssertTrue(app.buttons["Play Random Albums"].waitForExistence(timeout: 8))
    XCTAssertFalse(app.textFields["Find a command"].exists)
    app.buttons["About Play Random Albums"].tap()
    XCTAssertTrue(app.buttons["Play Station"].waitForExistence(timeout: 5))
    XCTAssertTrue(
      app.staticTexts.matching(NSPredicate(format: "label CONTAINS 'randomly chosen album'"))
        .firstMatch.exists)
    capture(app, "Radio Station")
    app.navigationBars.buttons.element(boundBy: 0).tap()
    app.buttons["Play Random Albums"].tap()
    XCTAssertTrue(app.navigationBars["Play Queue"].waitForExistence(timeout: 8))
    XCTAssertTrue(app.buttons["Queue Type"].exists)
    app.buttons["Queue Type"].tap()
    XCTAssertTrue(app.buttons["Manual Queue"].waitForExistence(timeout: 3))
    app.buttons["Manual Queue"].tap()
    capture(app, "Manual Queue Mode")
    app.buttons["Queue Type"].tap()
    app.buttons["Stay Close Mix"].tap()
    XCTAssertTrue(app.navigationBars["Stay Close Mix"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.buttons["Start track"].exists)
    XCTAssertFalse(app.navigationBars.buttons["Done"].exists)
    capture(app, "Inline Radio Configuration")

  }

  func testAccessibleColumnLayout() {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments = [
      "--silent", "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityM",
    ]
    app.launch()
    connectFixture(app)
    capture(app, "Large Text Library")
    app.buttons["Artists"].tap()
    XCTAssertTrue(app.staticTexts["A Artist"].waitForExistence(timeout: 10))
    let table = app.tables["library-column"]
    XCTAssertGreaterThan(table.cells.element(boundBy: 1).frame.height, 90)
    capture(app, "Large Text Artists")
    XCTAssertFalse(table.otherElements["Section index"].exists)
    table.swipeUp(velocity: .fast)
    capture(app, "Large Text Scrolling")

  }

  private func checkPalette(_ app: XCUIApplication) {
    let matches = app.buttons.matching(NSPredicate(format: "label == 'Command Palette'"))
    guard let palette = matches.allElementsBoundByIndex.first(where: \.isHittable) else {
      XCTFail("No accessible Command Palette button in the active presentation")
      return
    }
    palette.tap()
    XCTAssertTrue(app.textFields["Find a command"].waitForExistence(timeout: 5))
    app.navigationBars["Command Palette"].buttons["Done"].tap()
  }
  private func openNowPlaying(_ app: XCUIApplication) {
    app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Now Playing, '")).firstMatch.tap()
  }

  private func connectFixture(_ app: XCUIApplication, audioMuse: Bool = false) {
    allowFixtureNetwork()
    app.activate()
    let albums = app.buttons["sidebar-open-Albums"]
    if audioMuse || app.launchArguments.contains("--test-profile=fixture-validation"),
      albums.waitForExistence(timeout: 3), !app.buttons["Add Library"].exists { return }
    capture(app, "Fixture initial screen")
    let add = app.buttons["Add Library"]
    if add.exists {
      add.tap()
    } else {
      XCTAssertTrue(app.buttons["Command Palette"].firstMatch.waitForExistence(timeout: 10))
      app.buttons["Command Palette"].firstMatch.tap()
      app.buttons["palette-group:Textamp"].tap()
      app.buttons["Settings…"].tap()
    }
    let library = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Fixture Music'"))
      .firstMatch
    if !library.exists {
      let connection = app.buttons["Add Navidrome Library"]
      XCTAssertTrue(connection.waitForExistence(timeout: 5))
      connection.tap()
      let url = app.textFields["Server URL"]
      XCTAssertTrue(url.waitForExistence(timeout: 5))
      enter("http://127.0.0.1:48791", into: url)
      enter("fixture", into: app.textFields["Username"].firstMatch)
      enter("fixture-password", into: app.secureTextFields["Password"].firstMatch, secure: true)
      enter("Fixture Music", into: app.textFields["Name (optional)"])
      if audioMuse {
        app.swipeUp()
        enter("http://127.0.0.1:48791", into: app.textFields["AudioMuse URL"])
        enter("fixture", into: app.textFields["audiomuse-username"])
        let password = app.secureTextFields["audiomuse-password"]
        for _ in 0..<4 where !password.isHittable { app.collectionViews.firstMatch.swipeUp() }
        enter("fixture-password", into: password, secure: true)
      }
      app.swipeUp()
      app.buttons["Connect"].tap()
      if audioMuse {
        XCTAssertTrue(app.buttons["Connect"].waitForNonExistence(timeout: 15))
        app.terminate()
        app.launch()
        return
      }
      dismissIconNotice(XCUIApplication(bundleIdentifier: "com.apple.springboard"), timeout: 1)
      wait(
        for: [
          expectation(
            for: NSPredicate { _, _ in
              library.exists || (albums.exists && !app.buttons["Connect"].exists)
            }, evaluatedWith: app)
        ], timeout: 15)
      if !library.exists && albums.exists { return }
    }
    XCTAssertTrue(library.waitForExistence(timeout: 5))
    library.tap()
    if app.buttons["Make active"].exists {
      app.buttons["Make active"].tap()
      XCTAssertTrue(app.staticTexts["library-heading"].waitForExistence(timeout: 10))
      return
    }
    if !app.navigationBars["Settings"].exists {
      app.navigationBars.buttons.element(boundBy: 0).tap()
    }
    if app.buttons["Done"].exists { app.buttons["Done"].tap() }
  }

  private func allowFixtureNetwork() {
    let alert = XCUIApplication(bundleIdentifier: "com.apple.springboard").alerts.firstMatch
    if alert.waitForExistence(timeout: 1),
      alert.label.contains("Textamp"), alert.label.contains("local networks"),
      alert.buttons["Allow"].exists
    {
      alert.buttons["Allow"].tap()
    }
  }

  private func capture(_ app: XCUIApplication, _ name: String) {
    let image = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
    image.name = name
    image.lifetime = .keepAlways
    add(image)
  }

  private func enter(_ text: String, into field: XCUIElement, secure: Bool = false) {
    field.tap()
    for character in text { field.typeText(String(character)) }
    if !secure { XCTAssertEqual(field.value as? String, text) }
  }
}

