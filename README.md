# 🎮 StynPS PlaySelect

A modern, high-performance Linux controller remapper for the DualShock 4 and DualSense (PS5) controllers. Built with a native `egui` interface, it gives you total control over your controller without the bloat.

## 👋 A Quick Note from the Developer

Hey there! I want to be totally upfront with anyone using this. I am an absolute beginner in Rust, and this project is heavily vibecoded (AI-assisted). I started building this a while ago to solve my own needs on Linux, and I have been slowly iterating on it and releasing updates ever since. Please keep this in mind if you decide to look through the source code! I am learning as I go and really appreciate any constructive feedback.

## ✨ Features

*   ⚡ **Native Performance:** Built in Rust for low overhead and snappy response times.
*   💡 **Lightbar Control:** Set custom colors, adjust brightness, and even sync the lightbar with your battery level.
*   📳 **Advanced Rumble:** Fine-tune the large and small motors. Test the rumble instantly and forward in-game rumble to your real controller.
*   🖱️ **Touchpad and Gyro:** Use the touchpad as a mouse or enable gyro aiming with adjustable sensitivity.
*   🎮 **Steam Input Handover:** Seamlessly pass controller control over to Steam Input when a game launches, or keep it locked to StynPS.
*   🚀 **Auto-Profiles:** Automatically switch mapping profiles based on the active window or running game.
*   🎨 **Beautiful UI:** Features four distinct themes (Dark, Light, Minimal-Light, Minimal-Dark) with elegant typography.

## 📦 Installation

Right now, the easiest way to run StynPS is via the AppImage. 

1. Download the latest `.AppImage` from the Releases page.
2. Make it executable: `chmod +x StynPS-x86_64.AppImage`
3. Run it! 

Because the app needs to interact with hardware, you will need to install a udev rule for permissions. You can do this directly from the Settings tab inside the app.

## 🛠️ Building from Source

If you prefer to build from source, make sure you have the Rust toolchain installed:

```bash
git clone https://github.com/Frnkynstyn-IV/StynPS-PlaySelect-.git
cd StynPS-PlaySelect-
cargo build --release
```

### Latest Release: UI Overhaul

**Project Note:** Just a quick reminder that I am an absolute beginner in Rust and this project is heavily vibecoded (AI-assisted). I am learning as I go and appreciate any feedback on this ongoing journey!

**Added**
* ✨ **Luxury UI Typography:** Integrated the Playfair Display font to make page titles and sidebar navigation buttons feel premium and elegant.
* ✈️ **Telegram Link:** Added a quick access button to the official StynPS Telegram channel in the About page.
* 🔔 **Animated Error Banners:** Added smooth fade and collapse animations for the "No rumble" and "No lightbar" warnings. These now only appear for a few seconds after clicking the test buttons and are colored red in every theme.

**Changed**
* 🎨 **Complete UI Overhaul:** Refreshed the overall look for a much cleaner and more modern experience.
* ✏️ Updated the "Touchpad & Gyro" tab text to "Touchpad and Gyro" for better readability with the new luxury font.
* 🔤 Restyled the sidebar navigation buttons to use the new luxury font.
* 📏 Adjusted the text input boxes to perfectly match the shape and size of the pill buttons next to them.

**Fixed**
* 🐛 Fixed a visual bug where dropdown menus (the profile selector and device selector) had mismatched border shapes compared to adjacent buttons. They now all share the same 8px border radius.
* 🔧 Fixed a compiler warning regarding float type fallback (float_literal_f32_fallback) in the lightbar rendering code.

## 💬 Support and Community

If you run into any bugs or have feature requests, please open an issue on GitHub. 

Join our official Telegram channel for updates and to chat: https://t.me/StynPS

## 📄 License

MIT License.
