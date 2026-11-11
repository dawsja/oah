import XCTest

/// Drives the installed app in its demo: taps the composer so the keyboard comes up, types, and
/// saves what the screen shows (to SHOT_DIR). What it's for is the pictures: whether the
/// composer sits above the keyboard is looked at, not asserted.
final class KeyboardTests: XCTestCase {
  private func save(_ name: String) {
    let dir = ProcessInfo.processInfo.environment["SHOT_DIR"] ?? NSTemporaryDirectory()
    let png = XCUIScreen.main.screenshot().pngRepresentation
    try? png.write(to: URL(fileURLWithPath: dir).appendingPathComponent("\(name).png"))
  }

  func testTheComposerWithTheKeyboardUp() {
    let app = XCUIApplication(bundleIdentifier: "dev.oah.mobile")
    app.launchEnvironment = ["OAH_DEMO": "1", "OAH_ROUTE": "/chat/c-invoices"]
    app.launch()

    // The composer is the page's one text area.
    let box = app.textViews.firstMatch
    if box.waitForExistence(timeout: 30) {
      box.tap()
    } else {
      // Not found by kind: tap where it is, near the bottom.
      app.coordinate(withNormalizedOffset: CGVector(dx: 0.4, dy: 0.9)).tap()
    }
    sleep(2)
    save("20-keyboard")

    let keyboardUp = app.keyboards.firstMatch.waitForExistence(timeout: 5)
    if keyboardUp {
      app.typeText("Does the index need a migration?")
      sleep(1)
      save("21-keyboard-typed")
    }
    XCTAssertTrue(keyboardUp, "the keyboard came up")
  }
}
