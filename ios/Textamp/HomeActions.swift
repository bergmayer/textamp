import Observation
import UIKit

@MainActor
@Observable
final class HomeActions {
  static let shared = HomeActions()
  enum Action: String {
    case search
    case randomAlbumRadio = "random_album_radio"
  }
  var pending: Action?
  @discardableResult
  func accept(_ item: UIApplicationShortcutItem) -> Bool {
    switch item.type {
    case "local.textamp.ios.search": pending = .search
    case "local.textamp.ios.random-album-radio": pending = .randomAlbumRadio
    default: return false
    }
    return true
  }
}

final class HomeActionDelegate: NSObject, UIApplicationDelegate {
  func application(
    _ application: UIApplication, configurationForConnecting session: UISceneSession,
    options: UIScene.ConnectionOptions
  ) -> UISceneConfiguration {
    if let shortcut = options.shortcutItem { HomeActions.shared.accept(shortcut) }
    let configuration = UISceneConfiguration(name: nil, sessionRole: session.role)
    configuration.delegateClass = HomeActionSceneDelegate.self
    return configuration
  }
}

final class HomeActionSceneDelegate: NSObject, UIWindowSceneDelegate {
  func windowScene(
    _ windowScene: UIWindowScene, performActionFor shortcutItem: UIApplicationShortcutItem,
    completionHandler: @escaping (Bool) -> Void
  ) {
    completionHandler(HomeActions.shared.accept(shortcutItem))
  }
}
