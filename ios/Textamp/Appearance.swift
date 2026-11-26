import SwiftUI
import UIKit

extension AppTheme {
  var isBrushedAluminum: Bool { name == "brushed-aluminum" }

  // The mobile canvas uses a lighter finish so small labels remain legible.
  private static let aluminumTile: UIImage? = {
    guard let stock = UIImage(named: "BrushedAluminum") else { return nil }
    let size = CGSize(width: stock.size.width / 2, height: stock.size.height / 2)
    let bounds = CGRect(origin: .zero, size: size)
    let format = UIGraphicsImageRendererFormat()
    format.scale = 2
    format.opaque = true
    return UIGraphicsImageRenderer(size: size, format: format).image { ctx in
      UIColor(white: 0.94, alpha: 1).setFill()
      ctx.fill(bounds)
      stock.draw(in: bounds, blendMode: .normal, alpha: 0.62)
    }
  }()

  var baseFill: AnyShapeStyle {
    if isBrushedAluminum, let tile = Self.aluminumTile {
      return AnyShapeStyle(ImagePaint(image: Image(uiImage: tile)))
    }
    return AnyShapeStyle(bg)
  }

  var tableBackground: UIColor {
    if isBrushedAluminum, let tile = Self.aluminumTile { return UIColor(patternImage: tile) }
    return UIColor(bg)
  }
}

/// Keep UIKit-owned navigation chrome in the same palette as the SwiftUI content.
struct WindowAppearance: UIViewRepresentable {
  let theme: AppTheme

  final class AppearanceView: UIView {
    var theme = AppTheme()
    private var appliedTheme: AppTheme?
    private let styledBars = NSHashTable<UINavigationBar>.weakObjects()

    override func didMoveToWindow() {
      super.didMoveToWindow()
      applyAppearance()
    }

    func applyAppearance() {
      guard let window else { return }
      let style =
        theme.colorScheme.map { $0 == .dark ? UIUserInterfaceStyle.dark : .light } ?? .unspecified
      if window.overrideUserInterfaceStyle != style { window.overrideUserInterfaceStyle = style }
      if appliedTheme != theme {
        let foreground = theme.system ? nil : UIColor(theme.text)
        let attributes: [NSAttributedString.Key: Any]? = foreground.map { [.foregroundColor: $0] }
        UINavigationBar.appearance().titleTextAttributes = attributes
        UINavigationBar.appearance().largeTitleTextAttributes = attributes
        styledBars.removeAllObjects()
        appliedTheme = theme
      }
      func visit(_ controller: UIViewController) {
        if let navigation = controller as? UINavigationController,
          !styledBars.contains(navigation.navigationBar)
        {
          WindowAppearance.apply(theme, to: navigation.navigationBar)
          styledBars.add(navigation.navigationBar)
        }
        for child in controller.children { visit(child) }
        if let presented = controller.presentedViewController { visit(presented) }
      }
      if let root = window.rootViewController { visit(root) }
    }
  }

  static func apply(_ theme: AppTheme, to bar: UINavigationBar) {
    let foreground = theme.system ? nil : UIColor(theme.text)
    func attributes(_ original: [NSAttributedString.Key: Any]) -> [NSAttributedString.Key: Any] {
      var result = original
      result[.foregroundColor] = foreground
      return result
    }
    func appearance(_ original: UINavigationBarAppearance) -> UINavigationBarAppearance {
      let result = UINavigationBarAppearance(barAppearance: original)
      result.titleTextAttributes = attributes(original.titleTextAttributes)
      result.largeTitleTextAttributes = attributes(original.largeTitleTextAttributes)
      return result
    }
    bar.titleTextAttributes = attributes(bar.titleTextAttributes ?? [:])
    bar.largeTitleTextAttributes = attributes(bar.largeTitleTextAttributes ?? [:])
    bar.standardAppearance = appearance(bar.standardAppearance)
    bar.scrollEdgeAppearance = bar.scrollEdgeAppearance.map(appearance)
    bar.compactAppearance = bar.compactAppearance.map(appearance)
    bar.compactScrollEdgeAppearance = bar.compactScrollEdgeAppearance.map(appearance)
    bar.tintColor = UIColor(theme.tint)
  }

  func makeUIView(context: Context) -> AppearanceView {
    let view = AppearanceView()
    view.isUserInteractionEnabled = false
    view.isAccessibilityElement = false
    return view
  }
  func updateUIView(_ view: AppearanceView, context: Context) {
    view.theme = theme
    view.applyAppearance()
  }
}
