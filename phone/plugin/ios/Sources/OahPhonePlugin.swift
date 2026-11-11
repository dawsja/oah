// The iPhone's side of the oah phone app: the few things only iOS can do. The app's Rust
// (phone/src-tauri) calls these by name; Tauri runs each off the main thread.

import Foundation
import Network
import SwiftRs
import Tauri
import UIKit
import WebKit

struct AppearanceArgs: Decodable {
  /// "dark", "light", or "system" (follow the phone).
  let style: String
}

struct ImageArgs: Decodable {
  let name: String
  let data: String
}

struct BrowseArgs: Decodable {
  let seconds: Double?
}

struct PathArgs: Decodable {
  let path: String
}

struct Saved: Encodable {
  let saved: Bool
}

struct NearbyDesktop: Encodable {
  let name: String
  /// The desktop's `ip:port` list, comma-separated, as its TXT record gives it.
  let addrs: String
}

struct Nearby: Encodable {
  let found: [NearbyDesktop]
}

/// DNS-SD's kDNSServiceErr_PolicyDenied: Local Network access is off, or iOS is still asking.
let policyDenied: Int32 = -65570

class OahPhonePlugin: Plugin {
  @objc public override func load(webview: WKWebView) {
    DispatchQueue.main.async {
      // The page fills the whole screen and keeps clear of the notch and the home indicator
      // itself (CSS safe-area insets). Left to itself the scroll view insets it as well, and
      // the page comes out shorter than the screen.
      webview.scrollView.contentInsetAdjustmentBehavior = .never
      // An app, not a page: no rubber band around it.
      webview.scrollView.bounces = false
      // A swipe from the left edge goes back a screen (the screens are entries in the
      // webview's history).
      webview.allowsBackForwardNavigationGestures = true
    }
  }

  /// The window takes the app's theme, so the status bar's text stays readable over it.
  @objc public func appearance(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(AppearanceArgs.self)
    DispatchQueue.main.async {
      let style: UIUserInterfaceStyle =
        args.style == "dark" ? .dark : args.style == "light" ? .light : .unspecified
      self.manager.viewController?.view.window?.overrideUserInterfaceStyle = style
      invoke.resolve()
    }
  }

  /// Offers an image to the share sheet (Save Image, Save to Files, AirDrop…).
  @objc public func saveImage(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ImageArgs.self)
    guard let data = Data(base64Encoded: args.data) else {
      invoke.reject("Could not read the image")
      return
    }
    let name = (args.name as NSString).lastPathComponent
    let file = FileManager.default.temporaryDirectory.appendingPathComponent(
      name.isEmpty ? "image.png" : name)
    do {
      try data.write(to: file, options: .atomic)
    } catch {
      invoke.reject(error.localizedDescription)
      return
    }
    DispatchQueue.main.async {
      guard let controller = self.manager.viewController else {
        invoke.reject("Could not open the share sheet")
        return
      }
      let sheet = UIActivityViewController(activityItems: [file], applicationActivities: nil)
      sheet.popoverPresentationController?.sourceView = controller.view
      sheet.completionWithItemsHandler = { _, completed, _, _ in
        try? FileManager.default.removeItem(at: file)
        invoke.resolve(Saved(saved: completed))
      }
      controller.present(sheet, animated: true)
    }
  }

  @objc public func copyImage(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ImageArgs.self)
    guard let data = Data(base64Encoded: args.data), let image = UIImage(data: data) else {
      invoke.reject("Could not read the image")
      return
    }
    DispatchQueue.main.async {
      UIPasteboard.general.image = image
      invoke.resolve(Saved(saved: true))
    }
  }

  @objc public func haptic(_ invoke: Invoke) {
    DispatchQueue.main.async {
      UIImpactFeedbackGenerator(style: .light).impactOccurred()
      invoke.resolve()
    }
  }

  /// Looks for oah desktops on the local network (Bonjour, `_oah._tcp`) for a few seconds.
  /// The first time, iOS asks for Local Network access; until that's answered (and when the
  /// answer was no) nothing is found, so this waits a little longer before giving up.
  @objc public func browse(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(BrowseArgs.self)
    let seconds = args.seconds ?? 2.5
    let queue = DispatchQueue(label: "dev.oah.mobile.browse")
    let browser = NWBrowser(
      for: .bonjourWithTXTRecord(type: "_oah._tcp", domain: nil), using: NWParameters())
    var found: [String: NearbyDesktop] = [:]
    var denied = false
    var until = Date().addingTimeInterval(seconds)
    let giveUp = Date().addingTimeInterval(seconds + 8)

    browser.stateUpdateHandler = { state in
      switch state {
      case .ready:
        // Allowed just now: start the search's clock again.
        if denied {
          denied = false
          until = Date().addingTimeInterval(seconds)
        }
      case .waiting(let error):
        if case .dns(let code) = error, code == policyDenied {
          denied = true
        }
      default:
        break
      }
    }
    browser.browseResultsChangedHandler = { results, _ in
      for result in results {
        guard case .service(let service, _, _, _) = result.endpoint else { continue }
        var txt: [String: String] = [:]
        if case .bonjour(let record) = result.metadata {
          txt = record.dictionary
        }
        found[service] = NearbyDesktop(name: txt["name"] ?? service, addrs: txt["addrs"] ?? "")
      }
    }

    func check() {
      let now = Date()
      if now < giveUp && (denied || now < until) {
        queue.asyncAfter(deadline: .now() + 0.25) { check() }
        return
      }
      browser.cancel()
      if denied && found.isEmpty {
        invoke.reject("local-network-denied")
      } else {
        invoke.resolve(Nearby(found: Array(found.values)))
      }
    }

    browser.start(queue: queue)
    queue.asyncAfter(deadline: .now() + 0.25) { check() }
  }

  /// Keeps a file or folder (the speech model) out of iCloud and computer backups.
  @objc public func excludeFromBackup(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PathArgs.self)
    var url = URL(fileURLWithPath: args.path)
    var values = URLResourceValues()
    values.isExcludedFromBackup = true
    do {
      try url.setResourceValues(values)
      invoke.resolve()
    } catch {
      invoke.reject(error.localizedDescription)
    }
  }

  /// Opens oah's page in the Settings app (where Local Network access is switched on).
  @objc public func openSettings(_ invoke: Invoke) {
    DispatchQueue.main.async {
      if let url = URL(string: UIApplication.openSettingsURLString) {
        UIApplication.shared.open(url, options: [:])
      }
      invoke.resolve()
    }
  }
}

@_cdecl("init_plugin_oah_phone")
public func initPlugin() -> Plugin {
  return OahPhonePlugin()
}
