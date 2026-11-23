import SwiftUI
import UIKit
import XCTest

@testable import Textamp

@MainActor
final class AppearanceTests: XCTestCase {
  func testOnlyOverflowingColumnHeadingsScrollAfterTheDelay() async throws {
    try XCTSkipIf(UIAccessibility.isReduceMotionEnabled, "The marquee respects Reduce Motion")
    let scene = try XCTUnwrap(
      UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
    let window = UIWindow(windowScene: scene)
    let headings = VStack(spacing: 0) {
      ColumnHeading(title: "A playlist with a title that extends well beyond the column")
        .frame(height: 40)
      ColumnHeading(title: "Short").frame(height: 40)
    }.frame(width: 140, height: 80)
    let host = UIHostingController(rootView: headings)
    host.safeAreaRegions = []
    window.frame = CGRect(x: 0, y: 0, width: 140, height: 80)
    window.rootViewController = host
    window.isHidden = false
    defer { window.isHidden = true }
    host.view.layoutIfNeeded()

    func pixels(in rect: CGRect) throws -> Data {
      let image = UIGraphicsImageRenderer(size: rect.size).image { context in
        context.cgContext.translateBy(x: -rect.minX, y: -rect.minY)
        host.view.drawHierarchy(in: host.view.bounds, afterScreenUpdates: true)
      }
      return try XCTUnwrap(image.pngData())
    }

    let center = CGPoint(x: host.view.bounds.midX, y: host.view.bounds.midY)
    let first = CGRect(x: center.x - 70, y: center.y - 40, width: 140, height: 40)
    let second = first.offsetBy(dx: 0, dy: 40)
    try await Task.sleep(for: .milliseconds(250))
    let truncated = try pixels(in: first)
    let short = try pixels(in: second)
    try await Task.sleep(for: .seconds(1))
    XCTAssertEqual(try pixels(in: first), truncated)
    try await Task.sleep(for: .seconds(10))
    XCTAssertNotEqual(try pixels(in: first), truncated)
    XCTAssertEqual(try pixels(in: second), short)
  }

  func testAppleMusicSearchUsesOnlyAppHandoffs() async throws {
    let query = "Qzxv, live? #2 & Δ"
    var components = URLComponents()
    components.host = "music.apple.com"
    components.path = "/search"
    components.queryItems = [URLQueryItem(name: "term", value: query)]
    components.scheme = "https"
    let universal = try XCTUnwrap(components.url)
    let request = ExternalSearchRequest(label: "Apple Music", url: universal, webURL: universal, requiresApp: true)
    var attempts: [URL] = []
    let opened = await request.open { url, appOnly in
      attempts.append(url)
      XCTAssertTrue(appOnly)
      return false
    }
    XCTAssertFalse(opened)
    XCTAssertEqual(attempts, [universal])
    attempts.removeAll()
    let accepted = await request.open { url, _ in
      attempts.append(url)
      return true
    }
    XCTAssertTrue(accepted)
    XCTAssertEqual(attempts, [universal])
  }

  func testHomeShortcutAcceptsOnlyRegisteredAction() {
    let actions = HomeActions.shared
    defer { actions.pending = nil }
    XCTAssertFalse(
      actions.accept(UIApplicationShortcutItem(type: "unknown", localizedTitle: "Unknown")))
    XCTAssertNil(actions.pending)
    XCTAssertTrue(
      actions.accept(
        UIApplicationShortcutItem(
          type: "local.textamp.ios.random-album-radio", localizedTitle: "Play Random Albums")))
    XCTAssertEqual(actions.pending, .randomAlbumRadio)
    XCTAssertTrue(actions.accept(UIApplicationShortcutItem(type: "local.textamp.ios.search", localizedTitle: "Search")))
    XCTAssertEqual(actions.pending, .search)
    let shortcuts =
      Bundle.main.object(forInfoDictionaryKey: "UIApplicationShortcutItems") as? [[String: Any]]
    XCTAssertTrue(shortcuts?.contains { ($0["UIApplicationShortcutItemType"] as? String) == "local.textamp.ios.search" } == true)
    XCTAssertTrue(
      shortcuts?.contains {
        ($0["UIApplicationShortcutItemType"] as? String) == "local.textamp.ios.random-album-radio"
      } == true)
  }

  func testSystemTextResolvesLegiblyInLightDarkAndIncreasedContrast() {
    let theme = AppTheme()
    XCTAssertNil(theme.colorScheme)
    func rgba(_ color: UIColor, _ traits: UITraitCollection) -> [Double] {
      var r: CGFloat = 0
      var g: CGFloat = 0
      var b: CGFloat = 0
      var a: CGFloat = 0
      XCTAssertTrue(color.resolvedColor(with: traits).getRed(&r, green: &g, blue: &b, alpha: &a))
      return [Double(r), Double(g), Double(b), Double(a)]
    }
    func light(_ rgb: [Double]) -> Double {
      let linear = rgb.prefix(3).map { $0 <= 0.04045 ? $0 / 12.92 : pow(($0 + 0.055) / 1.055, 2.4) }
      return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
    }
    for style in [UIUserInterfaceStyle.light, .dark] {
      for contrast in [UIAccessibilityContrast.normal, .high] {
        let traits = UITraitCollection(traitsFrom: [
          UITraitCollection(userInterfaceStyle: style),
          UITraitCollection(accessibilityContrast: contrast),
        ])
        for ink in [theme.text, theme.muted] {
          let foreground = rgba(UIColor(ink), traits)
          for surface in [theme.bg, theme.card] {
            let background = rgba(UIColor(surface), traits)
            let mixed = (0..<3).map {
              foreground[$0] * foreground[3] + background[$0] * (1 - foreground[3])
            }
            let a = light(mixed)
            let b = light(background)
            XCTAssertGreaterThanOrEqual((max(a, b) + 0.05) / (min(a, b) + 0.05), 4.5)
          }
        }
      }
    }
  }

  func testNamedThemesApplyTheirTextToEveryNavigationAppearanceAndSystemRestoresDefaults() {
    let bar = UINavigationBar()
    let font = UIFont.systemFont(ofSize: 21, weight: .semibold)
    let appearance = UINavigationBarAppearance()
    appearance.configureWithTransparentBackground()
    appearance.titleTextAttributes = [.font: font]
    appearance.largeTitleTextAttributes = [.font: font]
    bar.standardAppearance = appearance
    bar.scrollEdgeAppearance = appearance
    bar.compactAppearance = appearance
    bar.compactScrollEdgeAppearance = appearance
    for (name, text, expected) in [
      ("nord", "#D8DEE9", [216, 222, 233]),
      ("platinum", "#000000", [0, 0, 0]),
      ("amber", "#FFB000", [255, 176, 0]),
    ] {
      WindowAppearance.apply(AppTheme(name: name, foreground: text), to: bar)
      for configuration in [
        bar.standardAppearance, bar.scrollEdgeAppearance!,
        bar.compactAppearance!, bar.compactScrollEdgeAppearance!,
      ] {
        for attributes in [
          configuration.titleTextAttributes, configuration.largeTitleTextAttributes,
        ] {
          guard let ink = attributes[.foregroundColor] as? UIColor else {
            XCTFail("Missing named navigation foreground for \(name)")
            continue
          }
          var red: CGFloat = 0
          var green: CGFloat = 0
          var blue: CGFloat = 0
          var alpha: CGFloat = 0
          XCTAssertTrue(ink.getRed(&red, green: &green, blue: &blue, alpha: &alpha))
          XCTAssertEqual([red, green, blue].map { Int(($0 * 255).rounded()) }, expected)
          XCTAssertEqual(alpha, 1)
          XCTAssertEqual(attributes[.font] as? UIFont, font)
        }
      }
    }
    WindowAppearance.apply(AppTheme(), to: bar)
    for configuration in [
      bar.standardAppearance, bar.scrollEdgeAppearance!,
      bar.compactAppearance!, bar.compactScrollEdgeAppearance!,
    ] {
      for attributes in [configuration.titleTextAttributes, configuration.largeTitleTextAttributes]
      {
        let ink = attributes[.foregroundColor] as? UIColor ?? .label
        for style in [UIUserInterfaceStyle.light, .dark] {
          let traits = UITraitCollection(userInterfaceStyle: style)
          XCTAssertEqual(ink.resolvedColor(with: traits), UIColor.label.resolvedColor(with: traits))
        }
      }
      XCTAssertEqual(configuration.titleTextAttributes[.font] as? UIFont, font)
    }
  }
}
