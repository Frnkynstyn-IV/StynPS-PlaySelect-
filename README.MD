# 🎮 StynPS (PlaySelect)

StynPS is a next-generation Linux controller remapper built entirely in Rust. It was born out of a desire to bring the powerful, user-friendly experience of *DS4Windows* to the Linux ecosystem—specifically for modern, performance-focused distributions like CachyOS.

Designed with a native PS5-inspired UI using egui, StynPS doesn't just work flawlessly; it looks good doing it.

## ✨ What Makes StynPS Different?

*   True Native Performance: Written in Rust, providing the low-latency input polling required for competitive gaming without the overhead of Python or Electron.
*   A UI That Adapts to You: 
    *   Dark Mode: Features a crisp White DualSense visualization.
    *   Light Mode: Swaps to a sleek Black DualSense visualization for perfect contrast.
    *   Curved, Fluid Interfaces: High border-radius elements and smooth animations for a modern feel.
*   Smart Auto-Profiles: Detects your active game window and automatically switches your controller layout, so you never have to alt-tab to change settings.
*   Full DualSense Support: Ready for the future with plans for adaptive trigger and haptic feedback integration.
*   Headless & Stable: The heavy lifting is done by a background daemon that communicates with the GUI. This means your controller keeps working even if you close the app.

## 🚀 Getting Started (AppImage)

StynPS is designed to be distro-agnostic. We are targeting a single AppImage build for maximum portability.

1.  Download the latest StynPS-x86_64.AppImage from the [Releases](https://github.com/Frnkynstyn-IV/StynPS-PlaySelect-/releases) page.
2.  Make it executable:
   
    chmod +x StynPS-x86_64.AppImage
    
3.  Run it!
    *Note: On first launch, StynPS will prompt you to install the necessary udev rules so it can read your controller (and create virtual devices) without needing sudo every time.*

## 🛠️ The Tech Stack

*   Language: [Rust](https://www.rust-lang.org/)
*   GUI Framework: [egui / eframe](https://github.com/emilk/egui)
*   Input Reading: evdev / hidraw
*   Virtual Output: uinput (with future plans for inputtino for advanced DualSense features)
*   Packaging: appimagetool / linuxdeploy (built inside a clean Distrobox container)

## 🙏 A Massive Thank You

This project would not exist without the incredible work of the [DS4Windows](https://github.com/Ryochan7/DS4Windows) developers. For years, they set the standard for what a controller remapper should be. StynPS is my attempt to bring that same level of polish and functionality to Linux.

## 🐛 Support & Contributions

Found a bug? Have an idea for a new feature? 
Please [open an issue](https://github.com/Frnkynstyn-IV/StynPS-PlaySelect-/issues) on GitHub. 

*(Current Status: Active Development. The backend daemon and Rust UI are being merged.)*
