# Task Switcher Redesign: Live Window Previews

## Project Goal
Replace the current text-based task switcher with a KDE-like interface featuring live window previews for a better user experience.

## Current Implementation Analysis

### Key Files and Their Roles

#### `/src/app.rs` - Main Application Logic
- **Alt+Tab Implementation**: Lines 76-81 define `LauncherTasks` enum with `AltTab` and `ShiftAltTab` commands
- **Window Switching State**: Line 154 contains `alt_tab: bool` field to track Alt+Tab mode
- **Message Handling**: Lines 168, 180-183 include messages for `AltTab`, `ShiftAltTab`, `AltRelease`, and `TabPress`
- **Window Navigation**: Lines 241-253 implement `focus_next()` and `focus_previous()` methods
- **Keyboard Event Handling**: Lines 1009-1012 listen for Alt key release events to activate selected window
- **Window Sorting**: Lines 494-504 contain special sorting logic for Alt+Tab mode that prioritizes actual windows over applications

#### `/src/subscriptions/launcher.rs` - Pop-Launcher Integration
- **Pop-Launcher Client**: Lines 45-95 manage IPC communication with pop-launcher service
- **Window Search**: Handles searching for open windows/applications via pop-launcher
- **Activation**: Manages window activation through the `Activate` request type
- **Service Management**: Maintains connection to pop-launcher for window enumeration and switching

#### `/src/components/list.rs` - Current UI Component
- Contains the existing list-based UI for displaying search results and windows

### Current Alt+Tab Flow
1. **Activation**: Alt+Tab triggers D-Bus activation, launcher switches to Alt+Tab mode (`alt_tab = true`)
2. **Window Enumeration**: Launcher requests search with empty string from pop-launcher, returns open windows
3. **Window Display**: Windows displayed in special Alt+Tab UI (no search input, different sorting)
4. **Navigation**: Tab key moves between windows, Shift+Tab moves backwards
5. **Selection**: Releasing Alt key activates currently focused window
6. **Window Priority**: Real windows (with `window` field) sorted before applications

### Pop-Launcher Integration Details

#### Dependencies (`Cargo.toml`)
```toml
pop-launcher = { git = "https://github.com/pop-os/launcher/" }
pop-launcher-service = { git = "https://github.com/pop-os/launcher/" }
```

#### Window Data Structure
- `SearchResult` from pop-launcher contains `window` field identifying actual open windows
- Window sorting logic (lines 500-504 in `app.rs`):
  ```rust
  list.sort_by(|a, b| {
      let a = i32::from(a.window.is_none());
      let b = i32::from(b.window.is_none());
      a.cmp(&b)
  });
  ```
- Window display logic (lines 729-733 in `app.rs`): For windows, description becomes title and name becomes subtitle

#### No Current Screenshot/Preview Functionality
The current implementation only handles:
- Window icons (via `IconSource`)
- Basic window information (name, description)
- Window existence checking (`item.window.is_some()`)

## Screenshot Technology Research

### Wayland Protocol Options

#### wlr-screencopy Protocol
- Available through `wayland-protocols-wlr` and `wayland-protocols` Rust crates
- Allows clients to ask compositor to copy screen content to client buffer
- Experimental protocol, backward incompatible changes possible
- Key components:
  - `ZwlrScreencopyFrameV1` - Handles copying frames to buffers
  - `zwlr_screencopy_manager_v1` - Main manager interface

#### Modern Wayland Protocols (2024)
- New protocols: `ext-image-capture-source-v1` and `ext-image-copy-capture-v1`
- Recently merged into Wayland Protocols repository
- Improved screen capture support

### Recommended Screenshot Library: XCap

#### Why XCap?
- **Cross-platform**: Linux (X11, Wayland), MacOS, Windows
- **Actively maintained**: Updated 1 week ago (as of analysis)
- **Window-specific capture**: Can capture individual windows
- **Simple API**: Easy integration

#### XCap Usage Example
```rust
use xcap::Window;

let windows = Window::all().unwrap();
for window in windows {
    if window.is_minimized().unwrap() {
        continue;
    }
    let image = window.capture_image().unwrap();
    image.save(format!("window-{}.png", window.title().unwrap())).unwrap();
}
```

#### Alternative Libraries
- **Screenshots**: Cross-platform, updated March 2024
- **Gazo**: Wayland-specific, last updated December 2022
- **Wayshot**: wlroots-specific, last updated December 2023

## Implementation Plan

### Phase 1: Foundation
1. **Add XCap dependency** to `Cargo.toml`
2. **Create preview component** in `src/components/preview_grid.rs`
3. **Design data structures** for screenshot caching and window correlation

### Phase 2: Screenshot Integration
1. **Implement window screenshot capture** using XCap
2. **Create window ID correlation** between pop-launcher and XCap windows
3. **Add screenshot caching system** with window ID mapping
4. **Handle screenshot updates** on window changes

### Phase 3: UI Implementation
1. **Design grid-based layout** similar to KDE task switcher
2. **Implement thumbnail rendering** (target: 256x144 aspect ratio)
3. **Add keyboard navigation** with arrow keys + Tab
4. **Implement selection highlighting**

### Phase 4: Integration
1. **Modify Alt+Tab mode** in `src/app.rs` to use preview UI
2. **Update message handling** for new navigation patterns
3. **Integrate with existing window activation** system
4. **Test cross-platform compatibility**

### Phase 5: Polish
1. **Optimize screenshot caching** for performance
2. **Add fade/transition effects** for better UX
3. **Handle edge cases** (minimized windows, multi-monitor)
4. **Performance tuning** and memory management

## Technical Considerations

### Window Correlation Challenge
- **Pop-launcher** provides `SearchResult` with window metadata
- **XCap** provides `Window` objects with capture capabilities
- Need to correlate these by window title, PID, or other identifiers

### Performance Considerations
- **Screenshot caching**: Avoid re-capturing on every navigation
- **Update strategy**: Periodic refresh vs. event-based updates
- **Memory management**: Limit cache size, cleanup old screenshots
- **Thumbnail size**: Balance quality vs. performance

### UI/UX Design
- **Grid layout**: 3-4 columns, adaptive to window count
- **Thumbnail size**: ~256x144 pixels (16:9 aspect ratio)
- **Navigation**: Arrow keys for 2D navigation, Tab for linear
- **Selection indicator**: Border highlight + slight scale
- **Window titles**: Below thumbnails, truncated if needed

## Files to Modify

### Core Implementation
- `Cargo.toml` - Add XCap dependency
- `src/app.rs` - Alt+Tab mode logic and preview UI integration
- `src/components/mod.rs` - Export new preview component
- `src/components/preview_grid.rs` - New grid-based preview component (to be created)

### Supporting Changes
- `src/subscriptions/launcher.rs` - Window data correlation (if needed)
- Message types and handling for new navigation patterns

## Next Steps
1. Add XCap dependency to `Cargo.toml`
2. Create basic preview grid component structure
3. Implement window screenshot capture functionality
4. Build window correlation system
5. Integrate with existing Alt+Tab mode

## Development Notes
- Current branch: `master`
- Git status: Clean working directory
- Consider creating feature branch for this work
- Test on multiple Wayland compositors (Sway, COSMIC, etc.)