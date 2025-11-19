#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use eframe::egui;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tray_icon::menu::{Menu, MenuItem, MenuEvent};
use std::env;
use std::sync::mpsc::{self, Sender, Receiver};

#[cfg(windows)]
use winreg::enums::*;
#[cfg(windows)]
use winreg::RegKey;

// 托盘消息类型
#[derive(Debug, Clone)]
enum TrayMessage {
    ShowWindow,
    Quit,
}

struct EmojiDrop {
    emoji: String,
    x: f32,
    y: f32,
    speed: f32,
}

#[derive(Serialize, Deserialize)]
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

struct RestReminderApp {
    state: AppState,
    config: AppConfig,
    start_time: Option<Instant>,
    time_remaining: Duration,

    work_input: String,
    rest_input: String,

    drops: Vec<EmojiDrop>,
    last_frame: Instant,

    should_fullscreen: bool,
    was_fullscreen: bool, // 跟踪上一次的全屏状态
    is_overlay_mode: bool, // 是否处于蒙层模式
    should_minimize: bool, // 是否应该最小化
    should_hide: bool, // 是否应该隐藏到托盘
    is_hidden: bool, // 是否已隐藏到托盘
    auto_start_enabled: bool, // 是否启用开机自启
    should_show_from_tray: bool, // 是否应该从托盘恢复显示
    tray_receiver: Option<Receiver<TrayMessage>>, // 托盘消息接收器
}

impl Default for RestReminderApp {
    fn default() -> Self {
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
            should_fullscreen: false,
            was_fullscreen: false,
            is_overlay_mode: false,
            should_minimize: false,
            should_hide: false,
            is_hidden: false,
            auto_start_enabled: check_auto_start(), // 检查当前是否已启用开机自启
            should_show_from_tray: false,
            tray_receiver: None,
        }
    }
}

impl RestReminderApp {
    fn start_work(&mut self) {
        self.state = AppState::Working;
        self.start_time = Some(Instant::now());
        self.time_remaining = Duration::from_secs(self.config.work_minutes * 60);
        self.drops.clear();
        self.should_fullscreen = false; // 工作时不全屏
        self.was_fullscreen = false; // 重置状态跟踪
        self.is_overlay_mode = false; // 禁用蒙层模式
    }

    fn start_rest(&mut self) {
        self.state = AppState::Resting;
        self.start_time = Some(Instant::now());
        self.time_remaining = Duration::from_secs(self.config.rest_minutes * 60);
        self.drops.clear();
        self.should_fullscreen = true; // 休息时全屏
        self.is_overlay_mode = true; // 启用蒙层模式
    }

    fn pause(&mut self) {
        if let Some(start) = self.start_time {
            let elapsed = start.elapsed();
            if elapsed < self.time_remaining {
                self.time_remaining -= elapsed;
            }
        }
        self.start_time = None;
        self.state = AppState::Paused;
        self.drops.clear();
        self.should_fullscreen = false; // 暂停时不全屏
        self.was_fullscreen = false; // 重置状态跟踪
        self.is_overlay_mode = false; // 禁用蒙层模式
    }

    fn tick(&mut self) {
        if let Some(start) = self.start_time {
            let elapsed = start.elapsed();

            if elapsed >= self.time_remaining {
                if self.state == AppState::Working {
                    self.start_rest();
                } else {
                    // 休息时间结束，设置最小化标志并暂停
                    self.should_minimize = true;
                    self.pause();
                }
            } else {
                self.time_remaining -= elapsed;
                self.start_time = Some(Instant::now());
            }
        }
    }

    fn format_time(&self) -> String {
        let total = self.time_remaining.as_secs();
        let min = total / 60;
        let sec = total % 60;
        format!("{:02}:{:02}", min, sec)
    }

    fn update_emojis(&mut self, ctx: &egui::Context) {
        let dt = self.last_frame.elapsed().as_secs_f32();
        self.last_frame = Instant::now();

        let screen = ctx.input(|i| i.screen_rect);
        let width = screen.width();

        if self.state == AppState::Resting {
            for _ in 0..2 {
                self.drops.push(EmojiDrop {
                    emoji: Self::random_emoji(),
                    x: fastrand::f32() * width,
                    y: -20.0,
                    speed: 80.0 + fastrand::f32() * 120.0,
                });
            }
        }

        for d in &mut self.drops {
            d.y += d.speed * dt;
        }

        self.drops.retain(|d| d.y < screen.bottom() + 50.0);
    }

    fn random_emoji() -> String {
        let list = ["😀", "😂", "😎", "🤩", "😭", "🔥", "🍓", "🍉", "💎", "✨", "🎉", "❤️"];
        list[fastrand::usize(..list.len())].to_string()
    }
}

impl eframe::App for RestReminderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick();
        ctx.request_repaint();

        // 确保启动时不是全屏状态（只在第一次运行时执行）
        static mut INITIALIZED: bool = false;
        unsafe {
            if !INITIALIZED {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                INITIALIZED = true;
            }
        }

        // 处理托盘消息通道
        if let Some(ref receiver) = self.tray_receiver {
            while let Ok(message) = receiver.try_recv() {
                match message {
                    TrayMessage::ShowWindow => {
                        // 恢复窗口显示
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        self.is_hidden = false;
                    }
                    TrayMessage::Quit => {
                        std::process::exit(0);
                    }
                }
            }
        }

        // 处理隐藏到托盘请求
        if self.should_hide {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            // 隐藏任务栏图标，只保留系统托盘图标
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.is_hidden = true;
            self.should_hide = false; // 重置标志
        }

        // 检查从托盘恢复显示请求（向后兼容）
        if self.should_show_from_tray {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.is_hidden = false;
            self.should_show_from_tray = false; // 重置标志
        }

        // 处理最小化请求
        if self.should_minimize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            self.should_minimize = false; // 重置标志
        }

        // 处理全屏状态切换 - 只在状态真正改变时发送命令
        if self.should_fullscreen != self.was_fullscreen {
            // 只在休息模式且确实是休息时间才启用全屏
            if self.is_overlay_mode && self.state == AppState::Resting {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));

                // 当进入休息模式时，让窗口获得焦点并居中显示
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                // 尝试让窗口更显眼一些
                if let Some(cmd) = egui::ViewportCommand::center_on_screen(ctx) {
                    ctx.send_viewport_cmd(cmd);
                }
            } else {
                // 确保非休息时间不是全屏
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }

            self.was_fullscreen = self.should_fullscreen;
        }

        self.update_emojis(ctx);

        // 绘制 emoji
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("emoji_layer"),
        ));

        let font = egui::FontId::proportional(40.0);

        for d in &self.drops {
            painter.text(
                egui::pos2(d.x, d.y),
                egui::Align2::CENTER_CENTER,
                &d.emoji,
                font.clone(),
                egui::Color32::WHITE,
            );
        }

        // 根据蒙层模式决定UI样式
        if self.is_overlay_mode {
            // 蒙层模式：显示半透明的休息提醒
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: egui::Color32::from_rgba_premultiplied(199, 237, 204, 120), // 护眼豆沙绿背景
                    inner_margin: egui::Margin::symmetric(50.0, 100.0),
                    ..Default::default()
                })
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(50.0);

                        ui.label(
                            egui::RichText::new("🌟 休息时间 🌟")
                                .size(64.0)
                                .color(egui::Color32::BLACK)
                        );

                        ui.add_space(30.0);

                        ui.label(
                            egui::RichText::new(self.format_time())
                                .size(96.0)
                                .color(egui::Color32::BLACK)
                                .strong()
                        );

                        ui.add_space(20.0);

                        ui.label(
                            egui::RichText::new("放松一下，活动活动身体")
                                .size(24.0)
                                .color(egui::Color32::from_rgba_premultiplied(0, 0, 0, 180))
                        );

                        ui.add_space(40.0);

                        if ui.button(
                            egui::RichText::new("提前结束休息")
                                .size(20.0)
                        ).clicked() {
                            // 最小化程序而不是恢复工作
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                            self.pause(); // 暂停计时器
                        }
                    });
                });
        } else {
            // 正常模式：显示完整界面
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("休息提醒助手");

                ui.add_space(10.0);

                ui.label(
                    egui::RichText::new(match self.state {
                        AppState::Working => "工作中...",
                        AppState::Resting => "休息中...",
                        AppState::Paused => "已暂停",
                    })
                    .size(24.0),
                );

                ui.add_space(20.0);

                ui.label(
                    egui::RichText::new(self.format_time())
                        .size(48.0)
                        .strong(),
                );

                ui.add_space(20.0);

                ui.horizontal(|ui| {
                    match self.state {
                        AppState::Paused => {
                            if ui.button("开始工作").clicked() {
                                self.start_work();
                            }
                        }
                        AppState::Working | AppState::Resting => {
                            if ui.button("暂停").clicked() {
                                self.pause();
                            }

                            if ui.button("跳过").clicked() {
                                if self.state == AppState::Working {
                                    self.start_rest();
                                } else {
                                    self.start_work();
                                }
                            }
                        }
                    }
                });

                ui.add_space(30.0);
                ui.separator();
                ui.heading("设置");

                ui.horizontal(|ui| {
                    ui.label("工作时长(分钟):");
                    ui.text_edit_singleline(&mut self.work_input);

                    if ui.button("确定").clicked() {
                        if let Ok(val) = self.work_input.parse::<u64>() {
                            if val > 0 {
                                self.config.work_minutes = val;
                            }
                        }
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("休息时长(分钟):");
                    ui.text_edit_singleline(&mut self.rest_input);

                    if ui.button("确定").clicked() {
                        if let Ok(val) = self.rest_input.parse::<u64>() {
                            if val > 0 {
                                self.config.rest_minutes = val;
                            }
                        }
                    }
                });

                ui.add_space(20.0);
                ui.separator();
                ui.heading("程序控制");

                ui.horizontal(|ui| {
                    if ui.button("隐藏到托盘").clicked() {
                        self.should_hide = true;
                    }

                    if ui.button("退出程序").clicked() {
                        std::process::exit(0);
                    }

                    // 在非休息时间显示关闭按钮
                    if self.state != AppState::Resting {
                        if ui.button("关闭窗口").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                });

                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    ui.label("开机自启:");
                    if ui.checkbox(&mut self.auto_start_enabled, "随系统启动").changed() {
                        let _ = toggle_auto_start(self.auto_start_enabled);
                    }
                });
            });
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    // 创建消息通道
    let (tray_sender, tray_receiver) = mpsc::channel::<TrayMessage>();

    // 创建托盘图标
    let _tray_icon = create_tray_icon(tray_sender).expect("Failed to create tray icon");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([400.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "休息提醒助手",
        options,
        Box::new(move |cc| {
            // 中文字体支持示例（可选）
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "cn".to_owned(),
                egui::FontData::from_static(include_bytes!("./fonts/NotoSansSC-VariableFont_wght.ttf")),
            );
            fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap()
                .insert(0, "cn".to_owned());

            cc.egui_ctx.set_fonts(fonts);

            let mut app = RestReminderApp::default();
            app.tray_receiver = Some(tray_receiver);
            Ok(Box::new(app))
        }),
    )
}

fn create_tray_icon(sender: Sender<TrayMessage>) -> Result<tray_icon::TrayIcon, Box<dyn std::error::Error>> {
    // 创建简单的图标（这里使用简单的路径，实际项目中可以使用图标文件）
    let icon_data = vec![255, 255, 255, 255, 0, 0, 0, 255];
    let icon_data_extended = icon_data.iter().cloned().cycle().take(1024).collect::<Vec<_>>();
    let icon = tray_icon::Icon::from_rgba(
        icon_data_extended,
        16,
        16,
    )?;

    // 先获取菜单事件接收器
    let menu_channel = MenuEvent::receiver();

    // 创建托盘菜单
    let menu = Menu::new();

    // 创建菜单项，使用自定义ID来区分
    let show_item = MenuItem::with_id("show", "显示窗口", true, None);
    let quit_item = MenuItem::with_id("quit", "退出", true, None);

    menu.append(&show_item)?;
    menu.append(&quit_item)?;

    // 创建托盘图标
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("休息提醒助手")
        .with_icon(icon)
        .build()?;

    std::thread::spawn(move || {
        loop {
            match menu_channel.recv() {
                Ok(event) => {
                    // 根据自定义ID来处理事件
                    match event.id.0.as_str() {
                        "show" => { // 显示窗口
                            let _ = sender.send(TrayMessage::ShowWindow);
                        }
                        "quit" => { // 退出
                            let _ = sender.send(TrayMessage::Quit);
                        }
                        _ => {
                            // 未知菜单项，忽略
                        }
                    }
                }
                Err(_) => {
                    break;
                }
            }
        }
    });

    Ok(tray)
}

// 为了使用TrayIconBuilder，需要添加导入
use tray_icon::TrayIconBuilder;

// 开机自启相关函数
#[cfg(windows)]
fn check_auto_start() -> bool {
    const REG_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
    const APP_NAME: &str = "RestReminder";

    match RegKey::predef(HKEY_CURRENT_USER).open_subkey(REG_KEY) {
        Ok(key) => {
            match key.get_value::<String, _>(APP_NAME) {
                Ok(_) => true,
                Err(_) => false,
            }
        }
        Err(_) => false,
    }
}

#[cfg(windows)]
fn toggle_auto_start(enable: bool) -> Result<(), Box<dyn std::error::Error>> {
    const REG_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
    const APP_NAME: &str = "RestReminder";

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = std::path::Path::new(REG_KEY);

    if enable {
        // 获取当前程序路径
        let current_exe = env::current_exe()?;
        let path_str = current_exe.to_string_lossy().to_string();

        // 打开或创建注册表项，并设置值
        match hkcu.create_subkey(path) {
            Ok((key, _disp)) => {
                key.set_value(APP_NAME, &path_str)?;
                println!("开机自启已启用: {}", path_str);
            }
            Err(e) => {
                return Err(format!("无法创建注册表项: {}", e).into());
            }
        }
    } else {
        // 移除开机自启
        match hkcu.open_subkey_with_flags(path, KEY_ALL_ACCESS) {
            Ok(key) => {
                match key.delete_value(APP_NAME) {
                    Ok(_) => println!("开机自启已禁用"),
                    Err(e) => {
                        // 如果值不存在，也算是禁用成功
                        if e.raw_os_error() == Some(2) { // ERROR_FILE_NOT_FOUND
                            println!("开机自启已禁用（值不存在）");
                        } else {
                            return Err(format!("无法删除注册表值: {}", e).into());
                        }
                    }
                }
            }
            Err(_) => {
                // 如果注册表项不存在，也算是禁用成功
                println!("开机自启已禁用（注册表项不存在）");
            }
        }
    }

    Ok(())
}

#[cfg(windows)]
fn get_auto_start_path() -> Result<String, Box<dyn std::error::Error>> {
    const REG_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
    const APP_NAME: &str = "RestReminder";

    match RegKey::predef(HKEY_CURRENT_USER).open_subkey(REG_KEY) {
        Ok(key) => {
            match key.get_value::<String, _>(APP_NAME) {
                Ok(path) => Ok(path),
                Err(_) => Err("注册表中未找到自启动项".into()),
            }
        }
        Err(e) => Err(format!("无法打开注册表项: {}", e).into()),
    }
}

#[cfg(windows)]
fn is_admin() -> bool {
    use winapi::um::securitybaseapi::GetTokenInformation;
    use winapi::um::processthreadsapi::GetCurrentProcess;
    use winapi::um::winnt::{TOKEN_QUERY, TokenElevation, TOKEN_ELEVATION, HANDLE};
    use std::ptr;

    unsafe {
        let mut token: HANDLE = ptr::null_mut();
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;

        // 获取当前进程令牌
        if winapi::um::processthreadsapi::OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY,
            &mut token,
        ) == 0 {
            return false;
        }

        // 获取令牌提升信息
        let result = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            size,
            &mut size,
        );

        winapi::um::handleapi::CloseHandle(token);

        result != 0 && elevation.TokenIsElevated != 0
    }
}

#[cfg(not(windows))]
fn check_auto_start() -> bool {
    false
}

#[cfg(not(windows))]
fn toggle_auto_start(_enable: bool) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
