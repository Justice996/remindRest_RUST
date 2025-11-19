# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a Rust-based **Rest Reminder Assistant** (休息提醒助手) application that implements the Pomodoro Technique. The project uses Rust edition 2024 and the egui GUI framework to create a cross-platform desktop application that helps users manage work/rest cycles.

## Key Features

- **Pomodoro Timer**: Configurable work and rest periods with automatic state switching
- **Smart Fullscreen Reminders**: Automatic fullscreen overlay when rest periods begin
- **Overlay Mode**: Semi-transparent fullscreen display that ensures visibility without being intrusive
- **Animated UI**: Dynamic emoji drop animations during rest periods
- **Auto-minimization**: Program automatically minimizes after rest periods end
- **User Control**: Pause, skip, or manually end rest periods
- **Persistent Settings**: Saves work/rest duration preferences

## Development Commands

### Building and Running
- `cargo build` - Build the project
- `cargo run` - Build and run the application
- `cargo check` - Check for compilation errors without building
- `cargo clippy` - Run linter checks for code quality verification

### Testing
- `cargo test` - Run all tests
- `cargo test <test_name>` - Run a specific test

### Other Useful Commands
- `cargo fmt` - Format code according to Rust standards
- `cargo clean` - Clean build artifacts
- `cargo build --release` - Build optimized release version

## Architecture

### Core Components

**Main Application Structure (`src/main.rs`)**:
- `RestReminderApp` - Main application state and logic
- `AppConfig` - Configuration for work/rest durations
- `AppState` - Enumeration for Working/Resting/Paused states
- `EmojiDrop` - Animation system for rest period visuals

**Key Systems**:
- **Timer Management**: High-precision timing using `std::time::Instant`
- **State Machine**: Automatic transitions between work, rest, and paused states
- **Animation Engine**: 60fps emoji drop system with physics simulation
- **UI Adaptation**: Dynamic interface switching between normal and overlay modes
- **Window Management**: Viewport commands for fullscreen, minimization, and focus control

### Dependencies

- **egui 0.28**: Immediate mode GUI framework
- **eframe 0.28**: Window management and application framework
- **serde**: Configuration serialization/deserialization
- **fastrand**: Random number generation for animations
- **chrono**: Time utilities
- **winapi**: Windows-specific window management (Windows only)

### File Structure
```
src/
├── main.rs          # Complete application implementation
├── fonts/           # Chinese font support files
└── Cargo.toml       # Project configuration and dependencies
```

## Application Flow

1. **Startup**: Application launches in paused state with default settings
2. **Work Period**: User starts work, timer counts down in normal window mode
3. **Auto Transition**: Work end triggers fullscreen overlay mode automatically
4. **Rest Period**: Semi-transparent overlay with large countdown timer and animations
5. **Rest End**: Automatic minimization when rest period completes
6. **User Control**: Manual pause/skip/early-end options available throughout

## UI Modes

**Normal Mode** (Work/Paused):
- Complete control panel with settings
- Timer display and control buttons
- Configuration options for work/rest durations

**Overlay Mode** (Rest):
- Fullscreen semi-transparent background
- Large countdown timer display
- Minimal controls (early rest end button)
- Animated emoji drops

## Development Notes

- **Thread Safety**: Application runs single-threaded with egui's event loop
- **Performance**: Optimized for minimal CPU usage during idle periods
- **Cross-Platform**: Works on Windows, macOS, and Linux (Windows-optimized)
- **Accessibility**: High contrast UI with large text for visibility
- **Responsive**: UI adapts to window resizing and different screen sizes

## Common Tasks

When working with this codebase:

1. **Adding New Features**: Extend the `RestReminderApp` struct and update UI in the `update()` method
2. **Modifying Timer Logic**: Update the `tick()` method and state transition functions
3. **UI Changes**: Modify the panel rendering sections in the `update()` method
4. **Animation Updates**: Enhance the `update_emojis()` method and `EmojiDrop` physics
5. **Configuration Changes**: Extend `AppConfig` struct and update serialization

The application is well-structured for extending with additional productivity features or visual enhancements.