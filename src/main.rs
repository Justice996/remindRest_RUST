#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use eframe::egui;
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};


// 新增的 winapi 引用
#[cfg(target_os = "windows")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
#[cfg(target_os = "windows")]
use winapi::shared::windef::HWND;
#[cfg(target_os = "windows")]
use winapi::um::winuser::{SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW};

#[cfg(target_os = "windows")]
use winreg::enums::*;
#[cfg(target_os = "windows")]
use winreg::RegKey;

// -------------------------
// 1. 强制开启控制台 (调试神器)
// -------------------------
#[cfg(target_os = "windows")]
fn attach_console() {
    // 确保这里的路径是正确的
    use winapi::um::consoleapi::AllocConsole;
    unsafe {
        let _ = AllocConsole();
    }
    println!("--- 控制台已附加，日志将显示在这里 ---");
}

#[cfg(not(target_os = "windows"))]
fn attach_console() {}

// -------------------------
// 2. 定义全局状态 (用于跨线程通信)
// -------------------------

static TRAY_SHOW_REQUEST: AtomicBool = AtomicBool::new(false);
static TRAY_QUIT_REQUEST: AtomicBool = AtomicBool::new(false);

// 用于存储窗口句柄的全局变量
static WINDOW_HANDLE: std::sync::atomic::AtomicPtr<std::ffi::c_void> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

#[derive(Debug, Clone)]
enum TrayMessage {
    MenuClick(String), // 菜单被点击 (show/quit)
    IconClick,         // 托盘图标本身被点击 (左键)
}

struct EmojiDrop {
    emoji: String,
    x: f32,
    y: f32,
    speed: f32,
}

#[derive(Serialize, Deserialize, Clone)]
struct AppConfig {
    work_minutes: u64,
    rest_minutes: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            work_minutes: 25,
            rest_minutes: 5,
        }
    }
}

#[derive(PartialEq, Debug)]
enum AppState {
    Working,
    Resting,
    Paused,
}

// -------------------------
// 3. App 主结构体
// -------------------------

struct RestReminderApp {
    state: AppState,
    config: AppConfig,
    start_time: Option<Instant>,
    time_remaining: Duration,
    
    work_input: String,
    rest_input: String,
    drops: Vec<EmojiDrop>,
    last_frame: Instant,

    is_initialized: bool,
    should_fullscreen: bool,
    was_fullscreen: bool,
    is_overlay_mode: bool,
    should_minimize: bool,
    should_hide: bool,
    
    should_show_from_tray: bool,
    auto_start_enabled: bool,
    should_quit: bool,

    tray_receiver: Receiver<TrayMessage>,
    // 必须持有这些对象，否则托盘图标会消失
    _tray_icon: TrayIcon,
    _tray_menu: Menu,
}

// -------------------------
// 4. 业务逻辑实现
// -------------------------

impl RestReminderApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        attach_console(); // 开启控制台
        setup_fonts(&cc.egui_ctx); // 设置字体

        let (tx, rx) = mpsc::channel();
        
        // 创建托盘
        let (tray_icon, tray_menu) = init_tray(tx, cc.egui_ctx.clone())
            .expect("无法创建托盘图标");

        let config = AppConfig::default();
        
        Self {
            state: AppState::Paused,
            start_time: None,
            time_remaining: Duration::from_secs(config.work_minutes * 60),
            work_input: config.work_minutes.to_string(),
            rest_input: config.rest_minutes.to_string(),
            config,
            drops: vec![],
            last_frame: Instant::now(),
            
            is_initialized: false,
            should_fullscreen: false,
            was_fullscreen: false,
            is_overlay_mode: false,
            should_minimize: false,
            should_hide: false,
            should_show_from_tray: false,
            auto_start_enabled: check_auto_start(),
            should_quit: false,

            tray_receiver: rx,
            _tray_icon: tray_icon,
            _tray_menu: tray_menu,
        }
    }

    fn start_work(&mut self) {
        self.state = AppState::Working;
        self.start_time = Some(Instant::now());
        self.time_remaining = Duration::from_secs(self.config.work_minutes * 60);
        self.drops.clear();
        self.should_fullscreen = false;
        self.is_overlay_mode = false;
    }

    fn start_rest(&mut self) {
        println!("开始休息模式，准备显示全屏蒙版");
        self.state = AppState::Resting;
        self.start_time = Some(Instant::now());
        self.time_remaining = Duration::from_secs(self.config.rest_minutes * 60);
        self.drops.clear();
        self.should_fullscreen = true;
        self.is_overlay_mode = true;

        // 确保窗口可见
        self.should_hide = false;
    }

    fn pause(&mut self) {
        if let Some(start) = self.start_time {
            let elapsed = start.elapsed();
            if elapsed < self.time_remaining {
                self.time_remaining -= elapsed;
            } else {
                self.time_remaining = Duration::ZERO;
            }
        }
        self.start_time = None;
        self.state = AppState::Paused;
        self.drops.clear();
        self.should_fullscreen = false;
        self.is_overlay_mode = false;
    }

    fn tick(&mut self) {
        if let Some(start) = self.start_time {
            let elapsed = start.elapsed();
            if elapsed >= self.time_remaining {
                if self.state == AppState::Working {
                    self.start_rest();
                } else if self.state == AppState::Resting {
                    self.should_minimize = true;
                    self.pause();
                    self.time_remaining = Duration::from_secs(self.config.work_minutes * 60);
                }
            } else {
                self.time_remaining -= elapsed;
                self.start_time = Some(Instant::now());
            }
        }
    }
    
    fn format_time(&self) -> String {
        let total = self.time_remaining.as_secs();
        format!("{:02}:{:02}", total / 60, total % 60)
    }

    fn update_emojis(&mut self, ctx: &egui::Context) {
        let dt = self.last_frame.elapsed().as_secs_f32();
        self.last_frame = Instant::now();
        let screen = ctx.input(|i| i.screen_rect);
        if self.state == AppState::Resting && fastrand::f32() < 0.1 {
             for _ in 0..2 {
                self.drops.push(EmojiDrop {
                    emoji: Self::random_emoji(),
                    x: fastrand::f32() * screen.width(),
                    y: -30.0,
                    speed: 100.0 + fastrand::f32() * 150.0,
                });
            }
        }
        for d in &mut self.drops { d.y += d.speed * dt; }
        self.drops.retain(|d| d.y < screen.bottom() + 50.0);
    }
    
    fn random_emoji() -> String {
        let list = ["😀", "😂", "😎", "🤩", "😭", "🔥", "🍓", "🍉", "💎", "✨", "🎉", "❤️", "🚀"];
        list[fastrand::usize(..list.len())].to_string()
    }

    fn process_tray_message(&mut self, msg: TrayMessage) {
        match msg {
            TrayMessage::MenuClick(id) => {
                match id.as_str() {
                    "show" => {
                        println!("处理显示窗口请求");
                        self.should_show_from_tray = true;
                    }
                    "quit" => {
                        println!("处理退出请求");
                        self.should_quit = true;
                    }
                    _ => {
                        println!("未知菜单ID: {}", id);
                    }
                }
            }
            TrayMessage::IconClick => {
                println!("处理托盘图标点击，显示窗口");
                self.should_show_from_tray = true;
            }
        }
    }

    // UI 渲染部分
    fn render_overlay(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame { fill: egui::Color32::from_rgba_premultiplied(200, 240, 210, 240), ..Default::default() })
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(100.0);
                    ui.label(egui::RichText::new("☕ 休息时间").size(60.0).color(egui::Color32::BLACK));
                    ui.label(egui::RichText::new(self.format_time()).size(100.0).strong().color(egui::Color32::BLACK));
                    ui.add_space(50.0);
                    if ui.button(egui::RichText::new("跳过休息").size(20.0)).clicked() {
                        self.should_minimize = true;
                        self.pause();
                        self.time_remaining = Duration::from_secs(self.config.work_minutes * 60);
                        // 确保退出覆盖模式
                        self.is_overlay_mode = false;
                        self.should_fullscreen = false;
                    }
                });
            });
    }

    fn render_main(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(20.0);
            let time_color = match self.state {
                AppState::Working => egui::Color32::from_rgb(200, 80, 80),
                AppState::Resting => egui::Color32::from_rgb(80, 180, 80),
                AppState::Paused => egui::Color32::GRAY,
            };
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(self.format_time()).size(60.0).color(time_color));
                ui.label(match self.state { AppState::Working => "🔥 专注中", AppState::Resting => "☕ 休息中", AppState::Paused => "⏸ 已暂停" });
            });
            ui.add_space(30.0);
            ui.horizontal(|ui| {
                ui.columns(3, |cols| {
                    if cols[0].button("开始专注").clicked() { self.start_work(); }
                    if cols[1].button("暂停").clicked() { self.pause(); }
                    if cols[2].button("休息一下").clicked() { self.start_rest(); }
                });
            });
            ui.separator();
            ui.collapsing("设置", |ui| {
                ui.horizontal(|ui| {
                    ui.label("专注时长(分):");
                    if ui.text_edit_singleline(&mut self.work_input).lost_focus() {
                        if let Ok(v) = self.work_input.parse() { self.config.work_minutes = v; }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("休息时长(分):");
                    if ui.text_edit_singleline(&mut self.rest_input).lost_focus() {
                        if let Ok(v) = self.rest_input.parse() { self.config.rest_minutes = v; }
                    }
                });
                // 修复了这里的调用错误
                ui.checkbox(&mut self.auto_start_enabled, "开机自启").changed().then(|| { 
                    let _ = toggle_auto_start(self.auto_start_enabled); 
                });
            });
            ui.add_space(20.0);
            if ui.button("隐藏到托盘").clicked() { self.should_hide = true; }
        });
    }

    // 修复了方法不存在的错误
    fn render_emojis(&self, ctx: &egui::Context) {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("emojis")));
        let font = egui::FontId::proportional(40.0);
        for d in &self.drops {
            painter.text(egui::pos2(d.x, d.y), egui::Align2::CENTER_CENTER, &d.emoji, font.clone(), egui::Color32::WHITE);
        }
    }
} // Impl 结束

// -------------------------
// 5. Eframe Update 实现
// -------------------------

impl eframe::App for RestReminderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {

        // 保存窗口句柄 (只需要保存一次)
        #[cfg(target_os = "windows")]
        {
            static INIT_HANDLE: std::sync::Once = std::sync::Once::new();
            INIT_HANDLE.call_once(|| {
                if let Ok(handle) = _frame.window_handle() {
                    if let RawWindowHandle::Win32(h) = handle.as_raw() {
                        let hwnd = h.hwnd.get() as *mut std::ffi::c_void;
                        WINDOW_HANDLE.store(hwnd, Ordering::SeqCst);
                        println!("保存窗口句柄: {:?}", hwnd);
                    }
                }
            });
        }

        // --- 0. 检查是否需要退出 ---
        if self.should_quit {
            println!("正在退出应用程序...");
            // 立即强制退出，避免任何延迟
            std::process::exit(0);
        }

        // --- 1. 检查托盘请求 (使用原子变量而不是消息通道) ---
        let mut handled_count = 0;

        // 检查显示窗口请求
        if TRAY_SHOW_REQUEST.load(Ordering::SeqCst) {
            println!("主界面检测到显示窗口请求");
            TRAY_SHOW_REQUEST.store(false, Ordering::SeqCst); // 重置标志
            self.should_show_from_tray = true;
            handled_count += 1;
        }

        // 检查退出请求
        if TRAY_QUIT_REQUEST.load(Ordering::SeqCst) {
            println!("主界面检测到退出请求");
            TRAY_QUIT_REQUEST.store(false, Ordering::SeqCst); // 重置标志
            self.should_quit = true;
            handled_count += 1;
        }

        if handled_count > 0 {
            println!("本轮处理了 {} 个托盘请求", handled_count);
        }

        // --- 2. 处理窗口关闭 -> 隐藏 ---
        if ctx.input(|i| i.viewport().close_requested()) && !self.should_quit {
            println!("用户点击关闭，转为隐藏模式");
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.should_hide = true;
        }

        // --- 3. 强制持续重绘和消息检查 ---
        // 始终强制重绘，确保托盘消息被处理
        ctx.request_repaint();
        ctx.request_repaint_after(Duration::from_millis(50)); // 20fps for tray message checking

        // --- 4. 状态刷新 ---
        match self.state {
            AppState::Resting => {
                self.update_emojis(ctx);
                ctx.request_repaint_after(Duration::from_millis(16)); // ~60fps for animations
            }
            AppState::Working => {
                ctx.request_repaint_after(Duration::from_millis(100)); // 更频繁的检查
            }
            AppState::Paused => {
                ctx.request_repaint_after(Duration::from_millis(50)); // 暂停状态也要频繁检查托盘消息
            }
        }
        self.tick();

        // --- 4. 执行窗口命令 ---

        if self.should_hide {
            println!("正在隐藏窗口到托盘...");
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));

            // 同时使用 Windows API 强制隐藏
            #[cfg(target_os = "windows")]
            {
                let hwnd = WINDOW_HANDLE.load(Ordering::SeqCst) as HWND;
                if !hwnd.is_null() {
                    unsafe {
                        use winapi::um::winuser::ShowWindow;
                        ShowWindow(hwnd, winapi::um::winuser::SW_HIDE);
                        println!("使用 Windows API 隐藏窗口: {:?}", hwnd);
                    }
                }
            }

            self.should_hide = false;
            println!("窗口隐藏完成");
        }

       if self.should_show_from_tray {
            println!("正在尝试唤醒窗口...");

            // 1. 基础 eframe 命令
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));

            // 2. 延迟一下再执行 Windows API 调用，确保窗口状态更新
            std::thread::sleep(Duration::from_millis(100));

            // 3. 使用 Windows API 强制操作
            #[cfg(target_os = "windows")]
            {
                if let Ok(handle) = _frame.window_handle() {
                    if let RawWindowHandle::Win32(h) = handle.as_raw() {
                        let hwnd = h.hwnd.get() as HWND;
                        println!("获取到窗口句柄: {:?}", hwnd);

                        unsafe {
                            // 先显示窗口
                            ShowWindow(hwnd, SW_RESTORE);
                            std::thread::sleep(Duration::from_millis(50));
                            // 然后置顶
                            let result = SetForegroundWindow(hwnd);
                            println!("SetForegroundWindow 结果: {}", result);
                        }
                    } else {
                        println!("不是 Win32 窗口句柄");
                    }
                } else {
                    println!("无法获取窗口句柄");
                }
            }

            // 4. 多次尝试获取焦点
            for i in 0..3 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
                std::thread::sleep(Duration::from_millis(100));
                println!("尝试获取焦点 {}/3", i + 1);
            }

            self.should_show_from_tray = false;
            println!("窗口显示逻辑执行完成");
        }

        if self.should_minimize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            self.should_minimize = false;
        }

        if !self.is_initialized {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            self.is_initialized = true;
        }
        if self.should_fullscreen != self.was_fullscreen {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.should_fullscreen));
            if self.should_fullscreen { ctx.send_viewport_cmd(egui::ViewportCommand::Focus); }
            self.was_fullscreen = self.should_fullscreen;
        }

        // --- 5. UI 渲染 ---
        if self.is_overlay_mode {
            self.render_overlay(ctx);
        } else {
            self.render_main(ctx);
        }
        if self.state == AppState::Resting {
            self.render_emojis(ctx);
        }
    }
}

// -------------------------
// 6. 辅助函数 (全局函数，必须放在 impl 外部)
// -------------------------

fn init_tray(_sender: Sender<TrayMessage>, ctx: egui::Context) -> Result<(TrayIcon, Menu), Box<dyn std::error::Error>> {
    // 创建一个更明显的托盘图标 - 番茄图标
    let mut icon_data = vec![0; 64 * 64 * 4]; // 64x64 RGBA
    for y in 0..64 {
        for x in 0..64 {
            let idx = (y * 64 + x) * 4;
            // 创建一个简单的番茄红色圆形图标
            let center_x = 32;
            let center_y = 32;
            let distance = ((x as i32 - center_x).pow(2) + (y as i32 - center_y).pow(2)) as f32;

            if distance <= 25.0 * 25.0 {
                // 红色圆形
                icon_data[idx] = 255;     // R
                icon_data[idx + 1] = 99;  // G
                icon_data[idx + 2] = 71;  // B
                icon_data[idx + 3] = 255; // A
            } else {
                // 透明背景
                icon_data[idx + 3] = 0;   // A
            }
        }
    }

    let icon = tray_icon::Icon::from_rgba(icon_data, 64, 64)?;

    let menu = Menu::new();
    menu.append(&MenuItem::with_id("show", "显示窗口", true, None))?;
    menu.append(&MenuItem::with_id("quit", "退出程序", true, None))?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu.clone()))
        .with_tooltip("番茄钟助手 - 点击显示窗口")
        .with_icon(icon)
        .build()?;

    // 启动托盘事件监听线程 (使用原子变量而不是消息通道)
    std::thread::spawn(move || {
        let menu_channel = MenuEvent::receiver();
        let tray_channel = TrayIconEvent::receiver();

        println!("托盘监听线程已启动...");

        loop {
            let mut event_handled = false;

            // 检查菜单点击事件
            if let Ok(event) = menu_channel.try_recv() {
                let id = event.id().0.clone();
                println!("后台线程捕获菜单事件: {}", id);

                match id.as_str() {
                    "show" => {
                        println!("直接处理显示窗口请求");
                        show_window_directly();
                        event_handled = true;
                    }
                    "quit" => {
                        println!("直接退出应用程序");
                        std::process::exit(0);
                    }
                    _ => {}
                }
            }

            // 检查托盘图标点击事件 (只处理左键点击，右键让系统显示菜单)
            if let Ok(event) = tray_channel.try_recv() {
                match event {
                    TrayIconEvent::Click { button, .. } => {
                        if button == tray_icon::MouseButton::Left {
                            println!("后台线程捕获图标左键点击事件，直接处理显示窗口请求");
                            show_window_directly();
                            event_handled = true;
                        } else {
                            println!("右键点击，让系统显示菜单");
                        }
                    }
                    TrayIconEvent::DoubleClick { button, .. } => {
                        if button == tray_icon::MouseButton::Left {
                            println!("后台线程捕获图标左键双击事件，直接处理显示窗口请求");
                            show_window_directly();
                            event_handled = true;
                        }
                    }
                    _ => {}
                }
            }

            // 如果处理了事件，触发重绘
            if event_handled {
                ctx.request_repaint();
            }

            std::thread::sleep(Duration::from_millis(50));
        }
        println!("托盘监听线程结束");
    });

    Ok((tray, menu))
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let font_path = "C:\\Windows\\Fonts\\msyh.ttc"; 
    if let Ok(font_data) = std::fs::read(font_path) {
        fonts.font_data.insert("system_ui".to_owned(), egui::FontData::from_owned(font_data));
        fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap().insert(0, "system_ui".to_owned());
        fonts.families.get_mut(&egui::FontFamily::Monospace).unwrap().push("system_ui".to_owned());
        ctx.set_fonts(fonts);
    }
}

#[cfg(target_os = "windows")]
fn check_auto_start() -> bool {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run")
        .and_then(|k| k.get_value::<String, _>("RestReminder")).is_ok()
}

#[cfg(target_os = "windows")]
fn toggle_auto_start(enable: bool) -> std::io::Result<()> {
    let key = RegKey::predef(HKEY_CURRENT_USER).create_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run")?.0;
    if enable {
        let path = std::env::current_exe()?;
        key.set_value("RestReminder", &path.to_string_lossy().as_ref())?;
    } else { let _ = key.delete_value("RestReminder"); }
    Ok(())
}

#[cfg(not(target_os = "windows"))] fn check_auto_start() -> bool { false }
#[cfg(not(target_os = "windows"))] fn toggle_auto_start(_: bool) -> std::io::Result<()> { Ok(()) }

// 直接显示窗口的函数 (在托盘线程中调用)
#[cfg(target_os = "windows")]
fn show_window_directly() {
    let hwnd = WINDOW_HANDLE.load(Ordering::SeqCst) as HWND;
    if !hwnd.is_null() {
        println!("直接调用 Windows API 显示窗口: {:?}", hwnd);
        unsafe {
            // 先显示窗口
            ShowWindow(hwnd, SW_SHOW);

            // 强制获取焦点和前台
            SetForegroundWindow(hwnd);

            // 额外：确保窗口不是全屏状态
            use winapi::um::winuser::{GetWindowLongPtrW, SetWindowLongPtrW, GWL_STYLE, GWL_EXSTYLE, WS_OVERLAPPEDWINDOW, WS_EX_APPWINDOW};

            // 获取当前样式
            let mut style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            let mut ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;

            // 确保有标题栏和边框
            style |= WS_OVERLAPPEDWINDOW;
            ex_style |= WS_EX_APPWINDOW;

            SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);

            // 最后再次确保窗口正常显示
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
    } else {
        println!("窗口句柄为空，无法直接显示");
    }
}

#[cfg(not(target_os = "windows"))]
fn show_window_directly() {
    println!("非 Windows 系统，不使用直接窗口调用");
}

// -------------------------
// 7. Main 入口 (必须在文件最底部)
// -------------------------

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([400.0, 550.0])
            .with_min_inner_size([300.0, 400.0])
            .with_close_button(true)
            .with_minimize_button(true)
            .with_maximize_button(false),
        ..Default::default()
    };
    eframe::run_native("番茄钟提醒", options, Box::new(|cc| Ok(Box::new(RestReminderApp::new(cc)))))
}