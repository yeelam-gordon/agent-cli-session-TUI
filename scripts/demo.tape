# vhs demo tape — produces docs/demo.gif
#
# This script drives the TUI through a representative flow for the README:
#   1. Launch into the active session list
#   2. Demonstrate / search
#   3. Toggle into Grouped view
#   4. Quit
#
# To record:
#   1. Install vhs:  https://github.com/charmbracelet/vhs
#                    (Windows: scoop install vhs   |   macOS: brew install vhs)
#   2. Make sure agent-session-tui is on PATH and your config.toml has at least
#      one provider with real sessions (otherwise the list is empty).
#   3. Run from repo root:
#         vhs scripts/demo.tape
#      Output: docs/demo.gif
#
# vhs interprets `Sleep` between actions; tune for your terminal speed.
# Each frame is exact — no manual editing needed.

Output docs/demo.gif

Set FontSize 14
Set Width 1200
Set Height 720
Set Theme "Catppuccin Mocha"
Set Padding 20
Set TypingSpeed 50ms

# Boot
Type "agent-session-tui"
Enter
Sleep 4s

# Browse a few sessions
Down
Sleep 500ms
Down
Sleep 500ms
Down
Sleep 500ms
Down@500ms 5

# Search for something
Type "/"
Sleep 500ms
Type "auth"
Sleep 2500ms

# Clear search
Escape
Sleep 1s

# Switch to grouped view
Tab@200ms 1
Sleep 500ms
Backspace
Sleep 200ms
# (Note: Shift+Tab — vhs sends as `Tab` followed by `Backspace` in some setups.
#  If your build expects literal Shift+Tab, replace the two lines above with:
#  Ctrl+Shift+Tab  — depends on terminal. Test once and tweak.)
Sleep 3s

# Quit cleanly
Type "q"
Sleep 1s
