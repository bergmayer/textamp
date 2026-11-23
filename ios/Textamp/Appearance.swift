import SwiftUI
import UIKit

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
