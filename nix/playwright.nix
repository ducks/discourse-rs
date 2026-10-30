{ pkgs }:

# The browser suite's runner (tests/browser, scripts/browser-test):
# nixpkgs' @playwright/test and the browsers its playwright-driver was
# built against. Both come from the same nixpkgs, so the test runner, the
# driver and the browser builds are one Playwright version; a runner from
# npm against these browsers (or the reverse) is the usual mismatch.
# Chromium only (the suite runs no other browser), in Playwright's
# "chromium" channel: the full build in its new headless mode, as the
# headless shell isn't in this set.
{
  buildInputs = [ pkgs.playwright-test ];

  shellHook = ''
    export PLAYWRIGHT_BROWSERS_PATH="${pkgs.playwright-driver.browsers-chromium}"
    export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
  '';
}
