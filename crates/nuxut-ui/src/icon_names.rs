//! Semantik ikon adları ve ikon setleri (ADR-0006) — adların ve glif eşlemesinin TEK yeri.
//!
//! İkon `IconName` ile istenir; seçili [`IconSet`] adı glife çevirir:
//! - `material`: Material Symbols Rounded ligatürü (dolgu `FILL` ekseniyle).
//! - `nerd`: Nerd Font Material Design Icons bloğu (`nf-md-*`) kod noktası; (çizgi, dolu) çifti.
//!
//! Yeni ad = aşağıdaki tabloya bir satır (dört alan da zorunlu; makro eksik alanı derletmez).
//! Nerd kod noktaları Nerd Fonts `glyphnames.json` (3.5.1) adlarından alınmıştır; satır
//! sonundaki yorum `nf-md-*` adlarıdır.

use std::str::FromStr;

use nuxut_theme::IconSet;

macro_rules! icons {
    ($($variant:ident $name:literal $material:literal $nerd:literal $nerd_filled:literal;)*) => {
        /// Semantik ikon adı. Dize biçimi (`as_str`, `FromStr`) snake_case'tir: `"volume_high"`.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum IconName {
            $($variant,)*
        }

        impl IconName {
            /// Tüm adlar (tablo sırası).
            pub const ALL: &'static [IconName] = &[$(Self::$variant,)*];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)*
                }
            }

            /// Material Symbols ligatür adı.
            fn material(self) -> &'static str {
                match self {
                    $(Self::$variant => $material,)*
                }
            }

            /// Nerd Font (çizgi, dolu) glifleri.
            fn nerd(self) -> (&'static str, &'static str) {
                match self {
                    $(Self::$variant => ($nerd, $nerd_filled),)*
                }
            }
        }
    };
}

icons! {
    Close "close" "close" "\u{f0156}" "\u{f0156}"; // nf-md-close / nf-md-close
    Check "check" "check" "\u{f012c}" "\u{f012c}"; // nf-md-check / nf-md-check
    Add "add" "add" "\u{f0415}" "\u{f0415}"; // nf-md-plus / nf-md-plus
    Remove "remove" "remove" "\u{f0374}" "\u{f0374}"; // nf-md-minus / nf-md-minus
    Search "search" "search" "\u{f0349}" "\u{f0349}"; // nf-md-magnify / nf-md-magnify
    Menu "menu" "menu" "\u{f035c}" "\u{f035c}"; // nf-md-menu / nf-md-menu
    MoreHoriz "more_horiz" "more_horiz" "\u{f01d8}" "\u{f01d8}"; // nf-md-dots_horizontal / nf-md-dots_horizontal
    MoreVert "more_vert" "more_vert" "\u{f01d9}" "\u{f01d9}"; // nf-md-dots_vertical / nf-md-dots_vertical
    ChevronLeft "chevron_left" "chevron_left" "\u{f0141}" "\u{f0141}"; // nf-md-chevron_left / nf-md-chevron_left
    ChevronRight "chevron_right" "chevron_right" "\u{f0142}" "\u{f0142}"; // nf-md-chevron_right / nf-md-chevron_right
    ChevronUp "chevron_up" "expand_less" "\u{f0143}" "\u{f0143}"; // nf-md-chevron_up / nf-md-chevron_up
    ChevronDown "chevron_down" "expand_more" "\u{f0140}" "\u{f0140}"; // nf-md-chevron_down / nf-md-chevron_down
    ArrowBack "arrow_back" "arrow_back" "\u{f004d}" "\u{f004d}"; // nf-md-arrow_left / nf-md-arrow_left
    ArrowForward "arrow_forward" "arrow_forward" "\u{f0054}" "\u{f0054}"; // nf-md-arrow_right / nf-md-arrow_right
    ArrowUpward "arrow_upward" "arrow_upward" "\u{f005d}" "\u{f005d}"; // nf-md-arrow_up / nf-md-arrow_up
    ArrowDownward "arrow_downward" "arrow_downward" "\u{f0045}" "\u{f0045}"; // nf-md-arrow_down / nf-md-arrow_down
    Refresh "refresh" "refresh" "\u{f0450}" "\u{f0450}"; // nf-md-refresh / nf-md-refresh
    Sync "sync" "sync" "\u{f04e6}" "\u{f04e6}"; // nf-md-sync / nf-md-sync
    History "history" "history" "\u{f02da}" "\u{f02da}"; // nf-md-history / nf-md-history
    Settings "settings" "settings" "\u{f08bb}" "\u{f0493}"; // nf-md-cog_outline / nf-md-cog
    Tune "tune" "tune" "\u{f062e}" "\u{f062e}"; // nf-md-tune / nf-md-tune
    Edit "edit" "edit" "\u{f0cb6}" "\u{f03eb}"; // nf-md-pencil_outline / nf-md-pencil
    Delete "delete" "delete" "\u{f09e7}" "\u{f01b4}"; // nf-md-delete_outline / nf-md-delete
    Save "save" "save" "\u{f0818}" "\u{f0193}"; // nf-md-content_save_outline / nf-md-content_save
    Copy "copy" "content_copy" "\u{f018f}" "\u{f018f}"; // nf-md-content_copy / nf-md-content_copy
    Cut "cut" "content_cut" "\u{f0190}" "\u{f0190}"; // nf-md-content_cut / nf-md-content_cut
    Paste "paste" "content_paste" "\u{f0192}" "\u{f0192}"; // nf-md-content_paste / nf-md-content_paste
    OpenInNew "open_in_new" "open_in_new" "\u{f03cc}" "\u{f03cc}"; // nf-md-open_in_new / nf-md-open_in_new
    Link "link" "link" "\u{f0337}" "\u{f0337}"; // nf-md-link / nf-md-link
    LinkOff "link_off" "link_off" "\u{f0338}" "\u{f0338}"; // nf-md-link_off / nf-md-link_off
    DragHandle "drag_handle" "drag_indicator" "\u{f01db}" "\u{f01db}"; // nf-md-drag / nf-md-drag
    Sort "sort" "sort" "\u{f04ba}" "\u{f04ba}"; // nf-md-sort / nf-md-sort
    Filter "filter" "filter_list" "\u{f0233}" "\u{f0232}"; // nf-md-filter_outline / nf-md-filter
    Swap "swap" "swap_horiz" "\u{f04e1}" "\u{f04e1}"; // nf-md-swap_horizontal / nf-md-swap_horizontal
    Fullscreen "fullscreen" "fullscreen" "\u{f0293}" "\u{f0293}"; // nf-md-fullscreen / nf-md-fullscreen
    Pin "pin" "push_pin" "\u{f0931}" "\u{f0403}"; // nf-md-pin_outline / nf-md-pin
    Tag "tag" "label" "\u{f04fc}" "\u{f04f9}"; // nf-md-tag_outline / nf-md-tag
    Bookmark "bookmark" "bookmark" "\u{f00c3}" "\u{f00c0}"; // nf-md-bookmark_outline / nf-md-bookmark
    Star "star" "star" "\u{f04d2}" "\u{f04ce}"; // nf-md-star_outline / nf-md-star
    Favorite "favorite" "favorite" "\u{f02d5}" "\u{f02d1}"; // nf-md-heart_outline / nf-md-heart
    CheckCircle "check_circle" "check_circle" "\u{f05e1}" "\u{f05e0}"; // nf-md-check_circle_outline / nf-md-check_circle
    CheckboxChecked "checkbox_checked" "check_box" "\u{f0132}" "\u{f0132}"; // nf-md-checkbox_marked
    CheckboxUnchecked "checkbox_unchecked" "check_box_outline_blank" "\u{f0131}" "\u{f0131}"; // nf-md-checkbox_blank_outline
    RadioChecked "radio_checked" "radio_button_checked" "\u{f043e}" "\u{f043e}"; // nf-md-radiobox_marked
    RadioUnchecked "radio_unchecked" "radio_button_unchecked" "\u{f043d}" "\u{f043d}"; // nf-md-radiobox_blank
    Info "info" "info" "\u{f02fd}" "\u{f02fc}"; // nf-md-information_outline / nf-md-information
    Warning "warning" "warning" "\u{f002a}" "\u{f0026}"; // nf-md-alert_outline / nf-md-alert
    Error "error" "error" "\u{f05d6}" "\u{f0028}"; // nf-md-alert_circle_outline / nf-md-alert_circle
    Help "help" "help" "\u{f0625}" "\u{f02d7}"; // nf-md-help_circle_outline / nf-md-help_circle
    Visibility "visibility" "visibility" "\u{f06d0}" "\u{f0208}"; // nf-md-eye_outline / nf-md-eye
    VisibilityOff "visibility_off" "visibility_off" "\u{f06d1}" "\u{f0209}"; // nf-md-eye_off_outline / nf-md-eye_off
    Logout "logout" "logout" "\u{f0343}" "\u{f0343}"; // nf-md-logout / nf-md-logout
    Power "power" "power_settings_new" "\u{f0425}" "\u{f0425}"; // nf-md-power / nf-md-power
    Restart "restart" "restart_alt" "\u{f0709}" "\u{f0709}"; // nf-md-restart / nf-md-restart
    Lock "lock" "lock" "\u{f0341}" "\u{f033e}"; // nf-md-lock_outline / nf-md-lock
    LockOpen "lock_open" "lock_open" "\u{f0340}" "\u{f033f}"; // nf-md-lock_open_outline / nf-md-lock_open
    Person "person" "person" "\u{f0013}" "\u{f0004}"; // nf-md-account_outline / nf-md-account
    Group "group" "group" "\u{f0b58}" "\u{f0849}"; // nf-md-account_group_outline / nf-md-account_group
    Apps "apps" "apps" "\u{f003b}" "\u{f003b}"; // nf-md-apps / nf-md-apps
    Home "home" "home" "\u{f06a1}" "\u{f02dc}"; // nf-md-home_outline / nf-md-home
    Folder "folder" "folder" "\u{f0256}" "\u{f024b}"; // nf-md-folder_outline / nf-md-folder
    File "file" "description" "\u{f09ee}" "\u{f0219}"; // nf-md-file_document_outline / nf-md-file_document
    Image "image" "image" "\u{f0976}" "\u{f02e9}"; // nf-md-image_outline / nf-md-image
    Download "download" "download" "\u{f0b8f}" "\u{f01da}"; // nf-md-download_outline / nf-md-download
    Upload "upload" "upload" "\u{f0e07}" "\u{f0552}"; // nf-md-upload_outline / nf-md-upload
    Share "share" "share" "\u{f1514}" "\u{f0497}"; // nf-md-share_variant_outline / nf-md-share_variant
    Send "send" "send" "\u{f1165}" "\u{f048a}"; // nf-md-send_outline / nf-md-send
    Chat "chat" "chat" "\u{f0ede}" "\u{f0b79}"; // nf-md-chat_outline / nf-md-chat
    Mail "mail" "mail" "\u{f01f0}" "\u{f01ee}"; // nf-md-email_outline / nf-md-email
    Terminal "terminal" "terminal" "\u{f018d}" "\u{f018d}"; // nf-md-console / nf-md-console
    Code "code" "code" "\u{f0174}" "\u{f0174}"; // nf-md-code_tags / nf-md-code_tags
    Palette "palette" "palette" "\u{f0e0c}" "\u{f03d8}"; // nf-md-palette_outline / nf-md-palette
    Wallpaper "wallpaper" "wallpaper" "\u{f0e09}" "\u{f0e09}"; // nf-md-wallpaper / nf-md-wallpaper
    Colorize "colorize" "colorize" "\u{f020a}" "\u{f020a}"; // nf-md-eyedropper / nf-md-eyedropper
    Screenshot "screenshot" "screenshot_monitor" "\u{f0e51}" "\u{f0e51}"; // nf-md-monitor_screenshot / nf-md-monitor_screenshot
    Videocam "videocam" "videocam" "\u{f0bdc}" "\u{f0567}"; // nf-md-video_outline / nf-md-video
    Camera "camera" "photo_camera" "\u{f0d5d}" "\u{f0100}"; // nf-md-camera_outline / nf-md-camera
    Print "print" "print" "\u{f1786}" "\u{f042a}"; // nf-md-printer_outline / nf-md-printer
    Cast "cast" "cast" "\u{f0118}" "\u{f0118}"; // nf-md-cast / nf-md-cast
    Desktop "desktop" "desktop_windows" "\u{f0379}" "\u{f0379}"; // nf-md-monitor / nf-md-monitor
    Monitor "monitor" "monitor" "\u{f0379}" "\u{f0379}"; // nf-md-monitor / nf-md-monitor (Ekranlar; Desktop ile aynı Nerd glifi, Material ayrı)
    Keyboard "keyboard" "keyboard" "\u{f097b}" "\u{f030c}"; // nf-md-keyboard_outline / nf-md-keyboard
    KeyboardCommand "keyboard_command" "keyboard_command_key" "\u{f0633}" "\u{f0633}"; // nf-md-apple_keyboard_command / nf-md-apple_keyboard_command
    // Ekran klavyesi tuşları (shell t125). Dolu = etkin (ör. basılı Shift).
    Backspace "backspace" "backspace" "\u{f0b5c}" "\u{f006e}"; // nf-md-backspace_outline / nf-md-backspace
    KeyboardReturn "keyboard_return" "keyboard_return" "\u{f0311}" "\u{f0311}"; // nf-md-keyboard_return / nf-md-keyboard_return
    Shift "shift" "shift" "\u{f0636}" "\u{f0737}"; // nf-md-apple_keyboard_shift / nf-md-arrow_up_bold
    ShiftLock "shift_lock" "shift_lock" "\u{f0060}" "\u{f005f}"; // nf-md-arrow_up_bold_circle_outline / nf-md-arrow_up_bold_circle (kilitli Shift; Shift okunun ailesi, CapsLock kutusundan ayrı)
    KeyboardCapslock "keyboard_capslock" "keyboard_capslock" "\u{f0739}" "\u{f0738}"; // nf-md-arrow_up_bold_box_outline / nf-md-arrow_up_bold_box (apple_keyboard_caps f0632 tuş boyunda bozuk okunuyordu; keyboard_caps f030e çok küçük — 2026-10-16)
    KeyboardTab "keyboard_tab" "keyboard_tab" "\u{f0312}" "\u{f0312}"; // nf-md-keyboard_tab / nf-md-keyboard_tab
    KeyboardHide "keyboard_hide" "keyboard_hide" "\u{f030f}" "\u{f030f}"; // nf-md-keyboard_close / nf-md-keyboard_close
    Mouse "mouse" "mouse" "\u{f037d}" "\u{f037d}"; // nf-md-mouse / nf-md-mouse
    Touchpad "touchpad" "touchpad_mouse" "\u{f07f8}" "\u{f07f8}"; // nf-md-trackpad / nf-md-trackpad
    Emoji "emoji" "mood" "\u{f01f2}" "\u{f0c68}"; // nf-md-emoticon_outline / nf-md-emoticon (emoji seçici kipi)
    Language "language" "language" "\u{f059f}" "\u{f059f}"; // nf-md-web / nf-md-web
    Accessibility "accessibility" "accessibility_new" "\u{f064c}" "\u{f064c}"; // nf-md-human_handsup / nf-md-human_handsup
    Update "update" "update" "\u{f06b0}" "\u{f06b0}"; // nf-md-update / nf-md-update
    Package "package" "package_2" "\u{f03d7}" "\u{f03d6}"; // nf-md-package_variant_closed / nf-md-package_variant
    Timer "timer" "timer" "\u{f051b}" "\u{f13ab}"; // nf-md-timer_outline / nf-md-timer
    Schedule "schedule" "schedule" "\u{f0150}" "\u{f0954}"; // nf-md-clock_outline / nf-md-clock
    CalendarToday "calendar_today" "calendar_today" "\u{f1a30}" "\u{f00f6}"; // nf-md-calendar_today_outline / nf-md-calendar_today
    CalendarMonth "calendar_month" "calendar_month" "\u{f0e18}" "\u{f0e17}"; // nf-md-calendar_month_outline / nf-md-calendar_month
    Event "event" "event" "\u{f0b66}" "\u{f00ee}"; // nf-md-calendar_blank_outline / nf-md-calendar_blank
    ViewDay "view_day" "calendar_view_day" "\u{f148a}" "\u{f056f}"; // nf-md-view_day_outline / nf-md-view_day
    ViewWeek "view_week" "view_week" "\u{f1491}" "\u{f0576}"; // nf-md-view_week_outline / nf-md-view_week
    Location "location" "location_on" "\u{f07d9}" "\u{f034e}"; // nf-md-map_marker_outline / nf-md-map_marker
    Notifications "notifications" "notifications" "\u{f009c}" "\u{f009a}"; // nf-md-bell_outline / nf-md-bell
    NotificationsOff "notifications_off" "notifications_off" "\u{f0a91}" "\u{f009b}"; // nf-md-bell_off_outline / nf-md-bell_off
    NotificationsAdd "notifications_add" "add_alert" "\u{f0a92}" "\u{f009d}"; // nf-md-bell_plus_outline / nf-md-bell_plus
    DoNotDisturb "do_not_disturb" "do_not_disturb_on" "\u{f0377}" "\u{f0376}"; // nf-md-minus_circle_outline / nf-md-minus_circle
    Wifi "wifi" "wifi" "\u{f05a9}" "\u{f05a9}"; // nf-md-wifi / nf-md-wifi
    WifiOff "wifi_off" "wifi_off" "\u{f05aa}" "\u{f05aa}"; // nf-md-wifi_off / nf-md-wifi_off
    Wifi1 "wifi_1" "network_wifi_1_bar" "\u{f091f}" "\u{f091f}"; // nf-md-wifi_strength_1 / nf-md-wifi_strength_1
    Wifi2 "wifi_2" "network_wifi_2_bar" "\u{f0922}" "\u{f0922}"; // nf-md-wifi_strength_2 / nf-md-wifi_strength_2
    Wifi3 "wifi_3" "network_wifi_3_bar" "\u{f0925}" "\u{f0925}"; // nf-md-wifi_strength_3 / nf-md-wifi_strength_3
    Wifi4 "wifi_4" "signal_wifi_4_bar" "\u{f0928}" "\u{f0928}"; // nf-md-wifi_strength_4 / nf-md-wifi_strength_4
    WifiLock "wifi_lock" "wifi_lock" "\u{f16bf}" "\u{f16bf}"; // nf-md-wifi_lock / nf-md-wifi_lock
    Ethernet "ethernet" "lan" "\u{f0200}" "\u{f0200}"; // nf-md-ethernet / nf-md-ethernet
    Vpn "vpn" "vpn_key" "\u{f0582}" "\u{f0582}"; // nf-md-vpn / nf-md-vpn
    Flight "flight" "flight" "\u{f001d}" "\u{f001d}"; // nf-md-airplane / nf-md-airplane
    Bluetooth "bluetooth" "bluetooth" "\u{f00af}" "\u{f00af}"; // nf-md-bluetooth / nf-md-bluetooth
    BluetoothConnected "bluetooth_connected" "bluetooth_connected" "\u{f00b1}" "\u{f00b1}"; // nf-md-bluetooth_connect / nf-md-bluetooth_connect
    BluetoothOff "bluetooth_off" "bluetooth_disabled" "\u{f00b2}" "\u{f00b2}"; // nf-md-bluetooth_off / nf-md-bluetooth_off
    Headphones "headphones" "headphones" "\u{f02cb}" "\u{f02cb}"; // nf-md-headphones / nf-md-headphones
    Speaker "speaker" "speaker" "\u{f04c3}" "\u{f04c3}"; // nf-md-speaker / nf-md-speaker
    Phone "phone" "smartphone" "\u{f011c}" "\u{f011c}"; // nf-md-cellphone / nf-md-cellphone (Bluetooth cihaz türü)
    Watch "watch" "watch" "\u{f0589}" "\u{f0589}"; // nf-md-watch / nf-md-watch (akıllı saat)
    Gamepad "gamepad" "sports_esports" "\u{f0eb7}" "\u{f0297}"; // nf-md-gamepad_variant_outline / nf-md-gamepad_variant
    Tablet "tablet" "tablet" "\u{f04f6}" "\u{f04f6}"; // nf-md-tablet / nf-md-tablet
    Mic "mic" "mic" "\u{f036e}" "\u{f036c}"; // nf-md-microphone_outline / nf-md-microphone
    MicOff "mic_off" "mic_off" "\u{f036d}" "\u{f036d}"; // nf-md-microphone_off / nf-md-microphone_off
    VolumeHigh "volume_high" "volume_up" "\u{f057e}" "\u{f057e}"; // nf-md-volume_high / nf-md-volume_high
    VolumeMedium "volume_medium" "volume_down" "\u{f0580}" "\u{f0580}"; // nf-md-volume_medium / nf-md-volume_medium
    VolumeLow "volume_low" "volume_mute" "\u{f057f}" "\u{f057f}"; // nf-md-volume_low / nf-md-volume_low
    VolumeMute "volume_mute" "volume_off" "\u{f0581}" "\u{f0581}"; // nf-md-volume_off / nf-md-volume_off
    Play "play" "play_arrow" "\u{f0f1b}" "\u{f040a}"; // nf-md-play_outline / nf-md-play
    Pause "pause" "pause" "\u{f03e4}" "\u{f03e4}"; // nf-md-pause / nf-md-pause
    Stop "stop" "stop" "\u{f04db}" "\u{f04db}"; // nf-md-stop / nf-md-stop
    SkipNext "skip_next" "skip_next" "\u{f0f27}" "\u{f04ad}"; // nf-md-skip_next_outline / nf-md-skip_next
    SkipPrevious "skip_previous" "skip_previous" "\u{f0f28}" "\u{f04ae}"; // nf-md-skip_previous_outline / nf-md-skip_previous
    Shuffle "shuffle" "shuffle" "\u{f049f}" "\u{f049f}"; // nf-md-shuffle_variant / nf-md-shuffle_variant
    Repeat "repeat" "repeat" "\u{f0456}" "\u{f0456}"; // nf-md-repeat / nf-md-repeat
    Music "music" "music_note" "\u{f0f74}" "\u{f0387}"; // nf-md-music_note_outline / nf-md-music_note
    Battery0 "battery_0" "battery_0_bar" "\u{f008e}" "\u{f008e}"; // nf-md-battery_outline / nf-md-battery_outline
    Battery10 "battery_10" "battery_1_bar" "\u{f007a}" "\u{f007a}"; // nf-md-battery_10 / nf-md-battery_10
    Battery20 "battery_20" "battery_1_bar" "\u{f007b}" "\u{f007b}"; // nf-md-battery_20 / nf-md-battery_20
    Battery30 "battery_30" "battery_2_bar" "\u{f007c}" "\u{f007c}"; // nf-md-battery_30 / nf-md-battery_30
    Battery40 "battery_40" "battery_3_bar" "\u{f007d}" "\u{f007d}"; // nf-md-battery_40 / nf-md-battery_40
    Battery50 "battery_50" "battery_3_bar" "\u{f007e}" "\u{f007e}"; // nf-md-battery_50 / nf-md-battery_50
    Battery60 "battery_60" "battery_4_bar" "\u{f007f}" "\u{f007f}"; // nf-md-battery_60 / nf-md-battery_60
    Battery70 "battery_70" "battery_4_bar" "\u{f0080}" "\u{f0080}"; // nf-md-battery_70 / nf-md-battery_70
    Battery80 "battery_80" "battery_5_bar" "\u{f0081}" "\u{f0081}"; // nf-md-battery_80 / nf-md-battery_80
    Battery90 "battery_90" "battery_6_bar" "\u{f0082}" "\u{f0082}"; // nf-md-battery_90 / nf-md-battery_90
    Battery100 "battery_100" "battery_full" "\u{f0079}" "\u{f0079}"; // nf-md-battery / nf-md-battery
    BatteryCharging "battery_charging" "battery_charging_full" "\u{f0084}" "\u{f0084}"; // nf-md-battery_charging / nf-md-battery_charging
    BatteryAlert "battery_alert" "battery_alert" "\u{f0083}" "\u{f0083}"; // nf-md-battery_alert / nf-md-battery_alert
    BatteryUnknown "battery_unknown" "battery_unknown" "\u{f0091}" "\u{f0091}"; // nf-md-battery_unknown / nf-md-battery_unknown
    PowerPlug "power_plug" "power" "\u{f1425}" "\u{f06a5}"; // nf-md-power_plug_outline / nf-md-power_plug
    PowerSaver "power_saver" "eco" "\u{f032a}" "\u{f032a}"; // nf-md-leaf / nf-md-leaf
    Balanced "balanced" "balance" "\u{f05d1}" "\u{f05d1}"; // nf-md-scale_balance / nf-md-scale_balance
    Performance "performance" "speed" "\u{f04c5}" "\u{f04c5}"; // nf-md-speedometer / nf-md-speedometer
    Brightness "brightness" "brightness_medium" "\u{f00df}" "\u{f00df}"; // nf-md-brightness_6 / nf-md-brightness_6
    BrightnessLow "brightness_low" "brightness_low" "\u{f00de}" "\u{f00de}"; // nf-md-brightness_5 / nf-md-brightness_5
    BrightnessHigh "brightness_high" "brightness_high" "\u{f00e0}" "\u{f00e0}"; // nf-md-brightness_7 / nf-md-brightness_7
    DarkMode "dark_mode" "dark_mode" "\u{f0594}" "\u{f0594}"; // nf-md-weather_night / nf-md-weather_night
    LightMode "light_mode" "light_mode" "\u{f05a8}" "\u{f05a8}"; // nf-md-white_balance_sunny / nf-md-white_balance_sunny
    NightLight "night_light" "nightlight" "\u{f1a4d}" "\u{f1a4c}"; // nf-md-lightbulb_night_outline / nf-md-lightbulb_night
    Coffee "coffee" "coffee" "\u{f06ca}" "\u{f0176}"; // nf-md-coffee_outline / nf-md-coffee
    Cpu "cpu" "memory" "\u{f0ee0}" "\u{f0ee0}"; // nf-md-cpu_64_bit / nf-md-cpu_64_bit
    Memory "memory" "memory_alt" "\u{f035b}" "\u{f035b}"; // nf-md-memory / nf-md-memory
    Gpu "gpu" "developer_board" "\u{f08ae}" "\u{f08ae}"; // nf-md-expansion_card / nf-md-expansion_card (Material Symbols'ta gpu yok)
    Disk "disk" "hard_drive" "\u{f02ca}" "\u{f02ca}"; // nf-md-harddisk / nf-md-harddisk
    Temperature "temperature" "thermostat" "\u{f050f}" "\u{f050f}"; // nf-md-thermometer / nf-md-thermometer
    Cloud "cloud" "cloud" "\u{f0163}" "\u{f015f}"; // nf-md-cloud_outline / nf-md-cloud
    WeatherClear "weather_clear" "clear_day" "\u{f0599}" "\u{f0599}"; // nf-md-weather_sunny / nf-md-weather_sunny
    WeatherNightClear "weather_night_clear" "clear_night" "\u{f0594}" "\u{f0594}"; // nf-md-weather_night / nf-md-weather_night
    WeatherPartlyCloudy "weather_partly_cloudy" "partly_cloudy_day" "\u{f0595}" "\u{f0595}"; // nf-md-weather_partly_cloudy / nf-md-weather_partly_cloudy
    WeatherCloudy "weather_cloudy" "cloud" "\u{f0590}" "\u{f0590}"; // nf-md-weather_cloudy / nf-md-weather_cloudy
    WeatherFog "weather_fog" "foggy" "\u{f0591}" "\u{f0591}"; // nf-md-weather_fog / nf-md-weather_fog
    WeatherRain "weather_rain" "rainy" "\u{f0597}" "\u{f0597}"; // nf-md-weather_rainy / nf-md-weather_rainy
    WeatherSnow "weather_snow" "weather_snowy" "\u{f0598}" "\u{f0598}"; // nf-md-weather_snowy / nf-md-weather_snowy
    WeatherThunder "weather_thunder" "thunderstorm" "\u{f0593}" "\u{f0593}"; // nf-md-weather_lightning / nf-md-weather_lightning
    Undo "undo" "undo" "\u{f054c}" "\u{f054c}"; // nf-md-undo
    Redo "redo" "redo" "\u{f044e}" "\u{f044e}"; // nf-md-redo
    Maximize "maximize" "crop_square" "\u{f05af}" "\u{f05af}"; // nf-md-window_maximize
    Restore "restore" "filter_none" "\u{f05b2}" "\u{f05b2}"; // nf-md-window_restore
    Minimize "minimize" "minimize" "\u{f05b0}" "\u{f05b0}"; // nf-md-window_minimize
    Animation "animation" "animation" "\u{f0a8f}" "\u{f05d8}"; // nf-md-animation_outline / nf-md-animation
    Panel "panel" "dock_to_bottom" "\u{f10a9}" "\u{f10a9}"; // nf-md-dock_bottom
    Font "font" "text_fields" "\u{f06d6}" "\u{f06d6}"; // nf-md-format_font
    Widgets "widgets" "widgets" "\u{f1355}" "\u{f072c}"; // nf-md-widgets_outline / nf-md-widgets
    Server "server" "dns" "\u{f048b}" "\u{f048b}"; // nf-md-server
    Work "work" "business_center" "\u{f0814}" "\u{f00d6}"; // nf-md-briefcase_outline / nf-md-briefcase
    AccountCircle "account_circle" "account_circle" "\u{f0b55}" "\u{f0009}"; // nf-md-account_circle_outline / nf-md-account_circle
    FolderOpen "folder_open" "folder_open" "\u{f0dcf}" "\u{f0770}"; // nf-md-folder_open_outline / nf-md-folder_open
    Grid "grid" "grid_view" "\u{f11d9}" "\u{f0570}"; // nf-md-view_grid_outline / nf-md-view_grid
    Window "window" "web_asset" "\u{f0614}" "\u{f08c6}"; // nf-md-application_outline / nf-md-application
    // Dosya seçici (shell filechooser). Üst klasör: ArrowUpward (ekran klavyesiyle ortak).
    ViewList "view_list" "view_list" "\u{f0572}" "\u{f0572}"; // nf-md-view_list / nf-md-view_list
    CreateNewFolder "create_new_folder" "create_new_folder" "\u{f0b9d}" "\u{f0257}"; // nf-md-folder_plus_outline / nf-md-folder_plus
    Usb "usb" "usb" "\u{f0553}" "\u{f0553}"; // nf-md-usb / nf-md-usb
}

impl IconName {
    /// Pil düzeyi (0–100) için ikon (10'luk kademeler).
    pub fn battery(percent: u8) -> Self {
        const LEVELS: [IconName; 11] = [
            IconName::Battery0,
            IconName::Battery10,
            IconName::Battery20,
            IconName::Battery30,
            IconName::Battery40,
            IconName::Battery50,
            IconName::Battery60,
            IconName::Battery70,
            IconName::Battery80,
            IconName::Battery90,
            IconName::Battery100,
        ];
        LEVELS[usize::from((percent.min(100) + 5) / 10)]
    }

    /// Wi-Fi sinyal gücü (0–100) için ikon.
    pub fn wifi(strength: u8) -> Self {
        match strength {
            0..=24 => Self::Wifi1,
            25..=49 => Self::Wifi2,
            50..=74 => Self::Wifi3,
            _ => Self::Wifi4,
        }
    }

    /// Ses düzeyi (0–100+) ve sessiz durumu için ikon.
    pub fn brightness(percent: u8) -> Self {
        match percent {
            0..=33 => Self::BrightnessLow,
            34..=66 => Self::Brightness,
            _ => Self::BrightnessHigh,
        }
    }

    pub fn volume(percent: u8, muted: bool) -> Self {
        match percent {
            _ if muted => Self::VolumeMute,
            0 => Self::VolumeMute,
            1..=33 => Self::VolumeLow,
            34..=66 => Self::VolumeMedium,
            _ => Self::VolumeHigh,
        }
    }
}

/// Bilinmeyen ikon adı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownIcon;

impl std::fmt::Display for UnknownIcon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("bilinmeyen ikon adı")
    }
}

impl std::error::Error for UnknownIcon {}

impl FromStr for IconName {
    type Err = UnknownIcon;

    /// Semantik ad; geçiş için Material Symbols ligatür adı da kabul edilir (`volume_up` →
    /// `VolumeHigh`). Semantik ad her zaman önce gelir.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        Self::ALL
            .iter()
            .find(|n| n.as_str() == s)
            .or_else(|| Self::ALL.iter().find(|n| n.material() == s))
            .copied()
            .ok_or(UnknownIcon)
    }
}

/// Adın `set`'teki glifi. `material`'de dolgu `FILL` ekseniyle yapılır (aynı ligatür);
/// `nerd`'de `filled` dolu glifi seçer.
pub fn glyph(set: IconSet, name: IconName, filled: bool) -> &'static str {
    match set {
        IconSet::Material => name.material(),
        IconSet::Nerd => {
            let (outline, solid) = name.nerd();
            if filled { solid } else { outline }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_roundtrip_and_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for &n in IconName::ALL {
            assert!(seen.insert(n.as_str()), "yinelenen ad {}", n.as_str());
            assert_eq!(n.as_str().parse::<IconName>(), Ok(n));
            assert!(n.as_str().bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
            for set in [IconSet::Material, IconSet::Nerd] {
                assert!(!glyph(set, n, false).is_empty() && !glyph(set, n, true).is_empty());
            }
            // Nerd glifleri tek kod noktası, Material Design Icons bloğunda (U+F0001–U+F1AF0).
            let (o, f) = n.nerd();
            for g in [o, f] {
                let mut cs = g.chars();
                let c = cs.next().unwrap() as u32;
                assert!(cs.next().is_none() && (0xF0001..=0xF1AF0).contains(&c), "{}: {c:x}", n.as_str());
            }
        }
    }

    /// Her ad, kuruluysa iki setin fontunda da gerçekten var: Material ligatürü tek glife,
    /// Nerd kod noktaları o fontun glifine şekillenir (yedek fonta düşmeden).
    #[test]
    fn glyphs_exist_in_installed_fonts() {
        let mut text = nuxut_render::TextSystem::new();
        let theme = nuxut_theme::Theme::default();
        let material = nuxut_theme::MATERIAL_SYMBOLS_FAMILY;
        let nerd = theme.typography.mono.family.as_str();
        // Denetim gerçekten ayırt ediyor mu: olmayan ligatür ve fontta olmayan kod noktası düşer.
        if text.has_family(material) {
            assert_ne!(text.glyphs_in_family(material, "yok_boyle_ikon_x"), Some(1));
        }
        if text.has_family(nerd) {
            assert_ne!(text.glyphs_in_family(nerd, "\u{10FFFD}"), Some(1));
        }
        let mut missing = Vec::new();
        for &n in IconName::ALL {
            if text.has_family(material) && text.glyphs_in_family(material, glyph(IconSet::Material, n, false)) != Some(1) {
                missing.push(format!("material {} ({})", n.as_str(), n.material()));
            }
            if text.has_family(nerd) {
                for filled in [false, true] {
                    if text.glyphs_in_family(nerd, glyph(IconSet::Nerd, n, filled)) != Some(1) {
                        missing.push(format!("nerd {} (dolu: {filled})", n.as_str()));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "fontta bulunmayan ikonlar:\n{}", missing.join("\n"));
    }

    #[test]
    fn legacy_ligature_names_resolve() {
        assert_eq!("volume_up".parse(), Ok(IconName::VolumeHigh));
        assert_eq!("expand_more".parse(), Ok(IconName::ChevronDown));
        // Semantik ad, aynı adlı ligatürden önce gelir.
        assert_eq!("cloud".parse(), Ok(IconName::Cloud));
        assert_eq!("memory".parse(), Ok(IconName::Memory));
        assert_eq!("yok_boyle_ikon".parse::<IconName>(), Err(UnknownIcon));
    }

    #[test]
    fn level_helpers() {
        assert_eq!(IconName::battery(0), IconName::Battery0);
        assert_eq!(IconName::battery(4), IconName::Battery0);
        assert_eq!(IconName::battery(50), IconName::Battery50);
        assert_eq!(IconName::battery(255), IconName::Battery100);
        assert_eq!(IconName::wifi(100), IconName::Wifi4);
        assert_eq!(IconName::volume(80, true), IconName::VolumeMute);
        assert_eq!(IconName::volume(20, false), IconName::VolumeLow);
        assert_eq!(IconName::brightness(10), IconName::BrightnessLow);
        assert_eq!(IconName::brightness(100), IconName::BrightnessHigh);
    }
}
