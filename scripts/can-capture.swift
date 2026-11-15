// Exits 0 when this process may capture the screen, 3 when macOS Screen
// Recording permission is missing (3, so a swift error isn't mistaken for it). Used by scripts/screenshot to fail early
// with a fix instead of screencapture's "could not create image from window".
// macOS grants the permission to the app that launched the shell (Terminal,
// iTerm, an editor, an agent host), so that app is the one to allow.
import CoreGraphics

exit(CGPreflightScreenCaptureAccess() ? 0 : 3)
