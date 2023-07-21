//! # Neotron Desktop BIOS
//!
//! Implement a Neotron BIOS as a Linux/Windows/macOS desktop application.
//!
//! The framebuffer is draw in a window. SD/MMC cards can be passed as files or block devices.

// -----------------------------------------------------------------------------
// Licence Statement
// -----------------------------------------------------------------------------
// Copyright (c) Jonathan 'theJPster' Pallant and the Neotron Developers, 2022
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE.  See the GNU General Public License for more
// details.
//
// You should have received a copy of the GNU General Public License along with
// this program.  If not, see <https://www.gnu.org/licenses/>.
// -----------------------------------------------------------------------------

// ===========================================================================
// Imports
// ===========================================================================

use std::io::prelude::*;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};

use clap::Parser;
use common::video::RGBColour;
use log::{debug, info};
use pix_engine::prelude::*;

use neotron_common_bios as common;

mod font;

// ===========================================================================
// Types
// ===========================================================================

struct MyApp {
	mode: common::video::Mode,
	font8x16: Vec<TextureId>,
	font8x8: Vec<TextureId>,
	sender: std::sync::mpsc::Sender<AppEvent>,
}

#[derive(Debug, PartialEq, Eq)]
enum AppEvent {
	Started,
	KeyUp(Key),
	KeyDown(Key),
}

/// Our video RAM
struct Framebuffer<const N: usize> {
	contents: std::cell::UnsafeCell<[u8; N]>,
}

/// A Desktop GUI version of a Neotron BIOS
#[derive(Parser)]
#[command(author, version, about)]
struct Args {
	/// Path to the OS library
	#[arg(long)]
	os: std::path::PathBuf,
	/// Path to a file to use as a disk image
	#[arg(long)]
	disk: Option<std::path::PathBuf>,
}

/// All our emulated hardware
struct Hardware {
	/// When we booted up
	boot_time: std::time::Instant,
	/// Our disk image
	disk_file: Option<std::fs::File>,
}

// ===========================================================================
// Global Variables
// ===========================================================================

/// We only have 'normal' sectored emulated disks
const BLOCK_SIZE: usize = 512;

/// The VRAM we share in a very hazardous way with the OS.
///
/// Big enough for 640x480 @ 256 colour.
// static mut FRAMEBUFFER: [u8; 307200] = [0u8; 307200];
static FRAMEBUFFER: Framebuffer<{ 640 * 480 }> = Framebuffer::new();

/// Scale the display to make it readable on a modern monitor
const SCALE_FACTOR: f32 = 2.0;

/// When we booted up
static HARDWARE: std::sync::Mutex<Option<Hardware>> = std::sync::Mutex::new(None);

/// The functions we export to the OS
static BIOS_API: common::Api = common::Api {
	api_version_get,
	bios_version_get,
	serial_get_info,
	serial_configure,
	serial_write,
	serial_read,
	time_clock_get,
	time_clock_set,
	configuration_get,
	configuration_set,
	video_is_valid_mode,
	video_mode_needs_vram,
	video_set_mode,
	video_get_mode,
	video_get_framebuffer,
	video_set_framebuffer,
	video_wait_for_line,
	memory_get_region,
	hid_get_event,
	hid_set_leds,
	video_get_palette,
	video_set_palette,
	video_set_whole_palette,
	i2c_bus_get_info,
	i2c_write_read,
	audio_mixer_channel_get_info,
	audio_mixer_channel_set_level,
	audio_output_set_config,
	audio_output_get_config,
	audio_output_data,
	audio_output_get_space,
	audio_input_set_config,
	audio_input_get_config,
	audio_input_data,
	audio_input_get_count,
	bus_select,
	bus_get_info,
	bus_write_read,
	bus_exchange,
	time_ticks_get,
	time_ticks_per_second,
	bus_interrupt_status,
	block_dev_get_info,
	block_dev_eject,
	block_write,
	block_read,
	block_verify,
	power_idle,
	power_control,
	compare_and_swap_bool,
};

/// Our standard 256 colour palette
static PALETTE: [AtomicU32; 256] = [
	// 0   Black (SYSTEM (#000000)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 0, 0).as_packed()),
	// 1   Maroon (SYSTEM (#800000)
	AtomicU32::new(common::video::RGBColour::from_rgb(128, 0, 0).as_packed()),
	// 2   Green (SYSTEM (#008000)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 128, 0).as_packed()),
	// 3   Olive (SYSTEM (#808000)
	AtomicU32::new(common::video::RGBColour::from_rgb(128, 128, 0).as_packed()),
	// 4   Navy (SYSTEM (#000080)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 0, 128).as_packed()),
	// 5   Purple (SYSTEM (#800080)
	AtomicU32::new(common::video::RGBColour::from_rgb(128, 0, 128).as_packed()),
	// 6   Teal (SYSTEM (#008080)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 128, 128).as_packed()),
	// 7   Silver (SYSTEM (#c0c0c0)
	AtomicU32::new(common::video::RGBColour::from_rgb(192, 192, 192).as_packed()),
	// 8   Grey (SYSTEM (#808080)
	AtomicU32::new(common::video::RGBColour::from_rgb(128, 128, 128).as_packed()),
	// 9   Red (SYSTEM (#ff0000)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 0, 0).as_packed()),
	// 10  Lime (SYSTEM (#00ff00)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 255, 0).as_packed()),
	// 11  Yellow (SYSTEM (#ffff00)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 255, 0).as_packed()),
	// 12  Blue (SYSTEM (#0000ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 0, 255).as_packed()),
	// 13  Fuchsia (SYSTEM (#ff00ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 0, 255).as_packed()),
	// 14  Aqua (SYSTEM (#00ffff)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 255, 255).as_packed()),
	// 15  White (SYSTEM (#ffffff)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 255, 255).as_packed()),
	// 16  Grey0 (#000000)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 0, 0).as_packed()),
	// 17  NavyBlue (#00005f)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 0, 95).as_packed()),
	// 18  DarkBlue (#000087)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 0, 135).as_packed()),
	// 19  Blue3 (#0000af)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 0, 175).as_packed()),
	// 20  Blue3 (#0000d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 0, 215).as_packed()),
	// 21  Blue1 (#0000ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 0, 255).as_packed()),
	// 22  DarkGreen (#005f00)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 95, 0).as_packed()),
	// 23  DeepSkyBlue4 (#005f5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 95, 95).as_packed()),
	// 24  DeepSkyBlue4 (#005f87)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 95, 135).as_packed()),
	// 25  DeepSkyBlue4 (#005faf)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 95, 175).as_packed()),
	// 26  DodgerBlue3 (#005fd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 95, 215).as_packed()),
	// 27  DodgerBlue2 (#005fff)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 95, 255).as_packed()),
	// 28  Green4 (#008700)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 135, 0).as_packed()),
	// 29  SpringGreen4 (#00875f)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 135, 95).as_packed()),
	// 30  Turquoise4 (#008787)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 135, 135).as_packed()),
	// 31  DeepSkyBlue3 (#0087af)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 135, 175).as_packed()),
	// 32  DeepSkyBlue3 (#0087d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 135, 215).as_packed()),
	// 33  DodgerBlue1 (#0087ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 135, 255).as_packed()),
	// 34  Green3 (#00af00)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 175, 0).as_packed()),
	// 35  SpringGreen3 (#00af5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 175, 95).as_packed()),
	// 36  DarkCyan (#00af87)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 175, 135).as_packed()),
	// 37  LightSeaGreen (#00afaf)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 175, 175).as_packed()),
	// 38  DeepSkyBlue2 (#00afd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 175, 215).as_packed()),
	// 39  DeepSkyBlue1 (#00afff)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 175, 255).as_packed()),
	// 40  Green3 (#00d700)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 215, 0).as_packed()),
	// 41  SpringGreen3 (#00d75f)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 215, 95).as_packed()),
	// 42  SpringGreen2 (#00d787)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 215, 135).as_packed()),
	// 43  Cyan3 (#00d7af)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 215, 175).as_packed()),
	// 44  DarkTurquoise (#00d7d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 215, 215).as_packed()),
	// 45  Turquoise2 (#00d7ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 215, 255).as_packed()),
	// 46  Green1 (#00ff00)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 255, 0).as_packed()),
	// 47  SpringGreen2 (#00ff5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 255, 95).as_packed()),
	// 48  SpringGreen1 (#00ff87)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 255, 135).as_packed()),
	// 49  MediumSpringGreen (#00ffaf)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 255, 175).as_packed()),
	// 50  Cyan2 (#00ffd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 255, 215).as_packed()),
	// 51  Cyan1 (#00ffff)
	AtomicU32::new(common::video::RGBColour::from_rgb(0, 255, 255).as_packed()),
	// 52  DarkRed (#5f0000)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 0, 0).as_packed()),
	// 53  DeepPink4 (#5f005f)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 0, 95).as_packed()),
	// 54  Purple4 (#5f0087)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 0, 135).as_packed()),
	// 55  Purple4 (#5f00af)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 0, 175).as_packed()),
	// 56  Purple3 (#5f00d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 0, 215).as_packed()),
	// 57  BlueViolet (#5f00ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 0, 255).as_packed()),
	// 58  Orange4 (#5f5f00)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 95, 0).as_packed()),
	// 59  Grey37 (#5f5f5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 95, 95).as_packed()),
	// 60  MediumPurple4 (#5f5f87)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 95, 135).as_packed()),
	// 61  SlateBlue3 (#5f5faf)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 95, 175).as_packed()),
	// 62  SlateBlue3 (#5f5fd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 95, 215).as_packed()),
	// 63  RoyalBlue1 (#5f5fff)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 95, 255).as_packed()),
	// 64  Chartreuse4 (#5f8700)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 135, 0).as_packed()),
	// 65  DarkSeaGreen4 (#5f875f)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 135, 95).as_packed()),
	// 66  PaleTurquoise4 (#5f8787)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 135, 135).as_packed()),
	// 67  SteelBlue (#5f87af)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 135, 175).as_packed()),
	// 68  SteelBlue3 (#5f87d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 135, 215).as_packed()),
	// 69  CornflowerBlue (#5f87ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 135, 255).as_packed()),
	// 70  Chartreuse3 (#5faf00)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 175, 0).as_packed()),
	// 71  DarkSeaGreen4 (#5faf5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 175, 95).as_packed()),
	// 72  CadetBlue (#5faf87)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 175, 135).as_packed()),
	// 73  CadetBlue (#5fafaf)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 175, 175).as_packed()),
	// 74  SkyBlue3 (#5fafd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 175, 215).as_packed()),
	// 75  SteelBlue1 (#5fafff)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 175, 255).as_packed()),
	// 76  Chartreuse3 (#5fd700)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 215, 0).as_packed()),
	// 77  PaleGreen3 (#5fd75f)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 215, 95).as_packed()),
	// 78  SeaGreen3 (#5fd787)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 215, 135).as_packed()),
	// 79  Aquamarine3 (#5fd7af)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 215, 175).as_packed()),
	// 80  MediumTurquoise (#5fd7d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 215, 215).as_packed()),
	// 81  SteelBlue1 (#5fd7ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 215, 255).as_packed()),
	// 82  Chartreuse2 (#5fff00)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 255, 0).as_packed()),
	// 83  SeaGreen2 (#5fff5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 255, 95).as_packed()),
	// 84  SeaGreen1 (#5fff87)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 255, 135).as_packed()),
	// 85  SeaGreen1 (#5fffaf)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 255, 175).as_packed()),
	// 86  Aquamarine1 (#5fffd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 255, 215).as_packed()),
	// 87  DarkSlateGray2 (#5fffff)
	AtomicU32::new(common::video::RGBColour::from_rgb(95, 255, 255).as_packed()),
	// 88  DarkRed (#870000)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 0, 0).as_packed()),
	// 89  DeepPink4 (#87005f)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 0, 95).as_packed()),
	// 90  DarkMagenta (#870087)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 0, 135).as_packed()),
	// 91  DarkMagenta (#8700af)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 0, 175).as_packed()),
	// 92  DarkViolet (#8700d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 0, 215).as_packed()),
	// 93  Purple (#8700ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 0, 255).as_packed()),
	// 94  Orange4 (#875f00)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 95, 0).as_packed()),
	// 95  LightPink4 (#875f5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 95, 95).as_packed()),
	// 96  Plum4 (#875f87)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 95, 135).as_packed()),
	// 97  MediumPurple3 (#875faf)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 95, 175).as_packed()),
	// 98  MediumPurple3 (#875fd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 95, 215).as_packed()),
	// 99  SlateBlue1 (#875fff)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 95, 255).as_packed()),
	// 100 Yellow4 (#878700)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 135, 0).as_packed()),
	// 101 Wheat4 (#87875f)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 135, 95).as_packed()),
	// 102 Grey53 (#878787)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 135, 135).as_packed()),
	// 103 LightSlateGrey (#8787af)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 135, 175).as_packed()),
	// 104 MediumPurple (#8787d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 135, 215).as_packed()),
	// 105 LightSlateBlue (#8787ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 135, 255).as_packed()),
	// 106 Yellow4 (#87af00)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 175, 0).as_packed()),
	// 107 DarkOliveGreen3 (#87af5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 175, 95).as_packed()),
	// 108 DarkSeaGreen (#87af87)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 175, 135).as_packed()),
	// 109 LightSkyBlue3 (#87afaf)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 175, 175).as_packed()),
	// 110 LightSkyBlue3 (#87afd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 175, 215).as_packed()),
	// 111 SkyBlue2 (#87afff)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 175, 255).as_packed()),
	// 112 Chartreuse2 (#87d700)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 215, 0).as_packed()),
	// 113 DarkOliveGreen3 (#87d75f)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 215, 95).as_packed()),
	// 114 PaleGreen3 (#87d787)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 215, 135).as_packed()),
	// 115 DarkSeaGreen3 (#87d7af)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 215, 175).as_packed()),
	// 116 DarkSlateGray3 (#87d7d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 215, 215).as_packed()),
	// 117 SkyBlue1 (#87d7ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 215, 255).as_packed()),
	// 118 Chartreuse1 (#87ff00)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 255, 0).as_packed()),
	// 119 LightGreen (#87ff5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 255, 95).as_packed()),
	// 120 LightGreen (#87ff87)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 255, 135).as_packed()),
	// 121 PaleGreen1 (#87ffaf)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 255, 175).as_packed()),
	// 122 Aquamarine1 (#87ffd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 255, 215).as_packed()),
	// 123 DarkSlateGray1 (#87ffff)
	AtomicU32::new(common::video::RGBColour::from_rgb(135, 255, 255).as_packed()),
	// 124 Red3 (#af0000)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 0, 0).as_packed()),
	// 125 DeepPink4 (#af005f)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 0, 95).as_packed()),
	// 126 MediumVioletRed (#af0087)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 0, 135).as_packed()),
	// 127 Magenta3 (#af00af)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 0, 175).as_packed()),
	// 128 DarkViolet (#af00d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 0, 215).as_packed()),
	// 129 Purple (#af00ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 0, 255).as_packed()),
	// 130 DarkOrange3 (#af5f00)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 95, 0).as_packed()),
	// 131 IndianRed (#af5f5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 95, 95).as_packed()),
	// 132 HotPink3 (#af5f87)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 95, 135).as_packed()),
	// 133 MediumOrchid3 (#af5faf)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 95, 175).as_packed()),
	// 134 MediumOrchid (#af5fd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 95, 215).as_packed()),
	// 135 MediumPurple2 (#af5fff)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 95, 255).as_packed()),
	// 136 DarkGoldenrod (#af8700)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 135, 0).as_packed()),
	// 137 LightSalmon3 (#af875f)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 135, 95).as_packed()),
	// 138 RosyBrown (#af8787)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 135, 135).as_packed()),
	// 139 Grey63 (#af87af)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 135, 175).as_packed()),
	// 140 MediumPurple2 (#af87d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 135, 215).as_packed()),
	// 141 MediumPurple1 (#af87ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 135, 255).as_packed()),
	// 142 Gold3 (#afaf00)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 175, 0).as_packed()),
	// 143 DarkKhaki (#afaf5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 175, 95).as_packed()),
	// 144 NavajoWhite3 (#afaf87)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 175, 135).as_packed()),
	// 145 Grey69 (#afafaf)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 175, 175).as_packed()),
	// 146 LightSteelBlue3 (#afafd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 175, 215).as_packed()),
	// 147 LightSteelBlue (#afafff)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 175, 255).as_packed()),
	// 148 Yellow3 (#afd700)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 215, 0).as_packed()),
	// 149 DarkOliveGreen3 (#afd75f)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 215, 95).as_packed()),
	// 150 DarkSeaGreen3 (#afd787)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 215, 135).as_packed()),
	// 151 DarkSeaGreen2 (#afd7af)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 215, 175).as_packed()),
	// 152 LightCyan3 (#afd7d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 215, 215).as_packed()),
	// 153 LightSkyBlue1 (#afd7ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 215, 255).as_packed()),
	// 154 GreenYellow (#afff00)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 255, 0).as_packed()),
	// 155 DarkOliveGreen2 (#afff5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 255, 95).as_packed()),
	// 156 PaleGreen1 (#afff87)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 255, 135).as_packed()),
	// 157 DarkSeaGreen2 (#afffaf)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 255, 175).as_packed()),
	// 158 DarkSeaGreen1 (#afffd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 255, 215).as_packed()),
	// 159 PaleTurquoise1 (#afffff)
	AtomicU32::new(common::video::RGBColour::from_rgb(175, 255, 255).as_packed()),
	// 160 Red3 (#d70000)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 0, 0).as_packed()),
	// 161 DeepPink3 (#d7005f)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 0, 95).as_packed()),
	// 162 DeepPink3 (#d70087)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 0, 135).as_packed()),
	// 163 Magenta3 (#d700af)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 0, 175).as_packed()),
	// 164 Magenta3 (#d700d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 0, 215).as_packed()),
	// 165 Magenta2 (#d700ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 0, 255).as_packed()),
	// 166 DarkOrange3 (#d75f00)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 95, 0).as_packed()),
	// 167 IndianRed (#d75f5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 95, 95).as_packed()),
	// 168 HotPink3 (#d75f87)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 95, 135).as_packed()),
	// 169 HotPink2 (#d75faf)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 95, 175).as_packed()),
	// 170 Orchid (#d75fd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 95, 215).as_packed()),
	// 171 MediumOrchid1 (#d75fff)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 95, 255).as_packed()),
	// 172 Orange3 (#d78700)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 135, 0).as_packed()),
	// 173 LightSalmon3 (#d7875f)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 135, 95).as_packed()),
	// 174 LightPink3 (#d78787)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 135, 135).as_packed()),
	// 175 Pink3 (#d787af)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 135, 175).as_packed()),
	// 176 Plum3 (#d787d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 135, 215).as_packed()),
	// 177 Violet (#d787ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 135, 255).as_packed()),
	// 178 Gold3 (#d7af00)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 175, 0).as_packed()),
	// 179 LightGoldenrod3 (#d7af5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 175, 95).as_packed()),
	// 180 Tan (#d7af87)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 175, 135).as_packed()),
	// 181 MistyRose3 (#d7afaf)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 175, 175).as_packed()),
	// 182 Thistle3 (#d7afd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 175, 215).as_packed()),
	// 183 Plum2 (#d7afff)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 175, 255).as_packed()),
	// 184 Yellow3 (#d7d700)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 215, 0).as_packed()),
	// 185 Khaki3 (#d7d75f)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 215, 95).as_packed()),
	// 186 LightGoldenrod2 (#d7d787)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 215, 135).as_packed()),
	// 187 LightYellow3 (#d7d7af)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 215, 175).as_packed()),
	// 188 Grey84 (#d7d7d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 215, 215).as_packed()),
	// 189 LightSteelBlue1 (#d7d7ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 215, 255).as_packed()),
	// 190 Yellow2 (#d7ff00)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 255, 0).as_packed()),
	// 191 DarkOliveGreen1 (#d7ff5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 255, 95).as_packed()),
	// 192 DarkOliveGreen1 (#d7ff87)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 255, 135).as_packed()),
	// 193 DarkSeaGreen1 (#d7ffaf)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 255, 175).as_packed()),
	// 194 Honeydew2 (#d7ffd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 255, 215).as_packed()),
	// 195 LightCyan1 (#d7ffff)
	AtomicU32::new(common::video::RGBColour::from_rgb(215, 255, 255).as_packed()),
	// 196 Red1 (#ff0000)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 0, 0).as_packed()),
	// 197 DeepPink2 (#ff005f)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 0, 95).as_packed()),
	// 198 DeepPink1 (#ff0087)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 0, 135).as_packed()),
	// 199 DeepPink1 (#ff00af)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 0, 175).as_packed()),
	// 200 Magenta2 (#ff00d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 0, 215).as_packed()),
	// 201 Magenta1 (#ff00ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 0, 255).as_packed()),
	// 202 OrangeRed1 (#ff5f00)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 95, 0).as_packed()),
	// 203 IndianRed1 (#ff5f5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 95, 95).as_packed()),
	// 204 IndianRed1 (#ff5f87)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 95, 135).as_packed()),
	// 205 HotPink (#ff5faf)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 95, 175).as_packed()),
	// 206 HotPink (#ff5fd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 95, 215).as_packed()),
	// 207 MediumOrchid1 (#ff5fff)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 95, 255).as_packed()),
	// 208 DarkOrange (#ff8700)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 135, 0).as_packed()),
	// 209 Salmon1 (#ff875f)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 135, 95).as_packed()),
	// 210 LightCoral (#ff8787)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 135, 135).as_packed()),
	// 211 PaleVioletRed1 (#ff87af)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 135, 175).as_packed()),
	// 212 Orchid2 (#ff87d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 135, 215).as_packed()),
	// 213 Orchid1 (#ff87ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 135, 255).as_packed()),
	// 214 Orange1 (#ffaf00)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 175, 0).as_packed()),
	// 215 SandyBrown (#ffaf5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 175, 95).as_packed()),
	// 216 LightSalmon1 (#ffaf87)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 175, 135).as_packed()),
	// 217 LightPink1 (#ffafaf)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 175, 175).as_packed()),
	// 218 Pink1 (#ffafd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 175, 215).as_packed()),
	// 219 Plum1 (#ffafff)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 175, 255).as_packed()),
	// 220 Gold1 (#ffd700)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 215, 0).as_packed()),
	// 221 LightGoldenrod2 (#ffd75f)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 215, 95).as_packed()),
	// 222 LightGoldenrod2 (#ffd787)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 215, 135).as_packed()),
	// 223 NavajoWhite1 (#ffd7af)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 215, 175).as_packed()),
	// 224 MistyRose1 (#ffd7d7)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 215, 215).as_packed()),
	// 225 Thistle1 (#ffd7ff)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 215, 255).as_packed()),
	// 226 Yellow1 (#ffff00)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 255, 0).as_packed()),
	// 227 LightGoldenrod1 (#ffff5f)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 255, 95).as_packed()),
	// 228 Khaki1 (#ffff87)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 255, 135).as_packed()),
	// 229 Wheat1 (#ffffaf)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 255, 175).as_packed()),
	// 230 Cornsilk1 (#ffffd7)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 255, 215).as_packed()),
	// 231 Grey100 (#ffffff)
	AtomicU32::new(common::video::RGBColour::from_rgb(255, 255, 255).as_packed()),
	// 232 Grey3 (#080808)
	AtomicU32::new(common::video::RGBColour::from_rgb(8, 8, 8).as_packed()),
	// 233 Grey7 (#121212)
	AtomicU32::new(common::video::RGBColour::from_rgb(18, 18, 18).as_packed()),
	// 234 Grey11 (#1c1c1c)
	AtomicU32::new(common::video::RGBColour::from_rgb(28, 28, 28).as_packed()),
	// 235 Grey15 (#262626)
	AtomicU32::new(common::video::RGBColour::from_rgb(38, 38, 38).as_packed()),
	// 236 Grey19 (#303030)
	AtomicU32::new(common::video::RGBColour::from_rgb(48, 48, 48).as_packed()),
	// 237 Grey23 (#3a3a3a)
	AtomicU32::new(common::video::RGBColour::from_rgb(58, 58, 58).as_packed()),
	// 238 Grey27 (#444444)
	AtomicU32::new(common::video::RGBColour::from_rgb(68, 68, 68).as_packed()),
	// 239 Grey30 (#4e4e4e)
	AtomicU32::new(common::video::RGBColour::from_rgb(78, 78, 78).as_packed()),
	// 240 Grey35 (#585858)
	AtomicU32::new(common::video::RGBColour::from_rgb(88, 88, 88).as_packed()),
	// 241 Grey39 (#626262)
	AtomicU32::new(common::video::RGBColour::from_rgb(98, 98, 98).as_packed()),
	// 242 Grey42 (#6c6c6c)
	AtomicU32::new(common::video::RGBColour::from_rgb(108, 108, 108).as_packed()),
	// 243 Grey46 (#767676)
	AtomicU32::new(common::video::RGBColour::from_rgb(118, 118, 118).as_packed()),
	// 244 Grey50 (#808080)
	AtomicU32::new(common::video::RGBColour::from_rgb(128, 128, 128).as_packed()),
	// 245 Grey54 (#8a8a8a)
	AtomicU32::new(common::video::RGBColour::from_rgb(138, 138, 138).as_packed()),
	// 246 Grey58 (#949494)
	AtomicU32::new(common::video::RGBColour::from_rgb(148, 148, 148).as_packed()),
	// 247 Grey62 (#9e9e9e)
	AtomicU32::new(common::video::RGBColour::from_rgb(158, 158, 158).as_packed()),
	// 248 Grey66 (#a8a8a8)
	AtomicU32::new(common::video::RGBColour::from_rgb(168, 168, 168).as_packed()),
	// 249 Grey70 (#b2b2b2)
	AtomicU32::new(common::video::RGBColour::from_rgb(178, 178, 178).as_packed()),
	// 250 Grey74 (#bcbcbc)
	AtomicU32::new(common::video::RGBColour::from_rgb(188, 188, 188).as_packed()),
	// 251 Grey78 (#c6c6c6)
	AtomicU32::new(common::video::RGBColour::from_rgb(198, 198, 198).as_packed()),
	// 252 Grey82 (#d0d0d0)
	AtomicU32::new(common::video::RGBColour::from_rgb(208, 208, 208).as_packed()),
	// 253 Grey85 (#dadada)
	AtomicU32::new(common::video::RGBColour::from_rgb(218, 218, 218).as_packed()),
	// 254 Grey89 (#e4e4e4)
	AtomicU32::new(common::video::RGBColour::from_rgb(228, 228, 228).as_packed()),
	// 255 Grey93 (#eeeeee)
	AtomicU32::new(common::video::RGBColour::from_rgb(238, 238, 238).as_packed()),
];

static VIDEO_MODE: AtomicU8 = AtomicU8::new(0);

static EV_QUEUE: std::sync::Mutex<Option<std::sync::mpsc::Receiver<AppEvent>>> =
	std::sync::Mutex::new(None);

// ===========================================================================
// Macros
// ===========================================================================

// None

// ===========================================================================
// Functions
// ===========================================================================

/// The entry point to our program.
///
/// We set up a game window using PixEngine. The event loop pumps in this thread.
///
/// We then load the OS from the `so` file given, and jump to it in a new thread.
fn main() {
	env_logger::init();

	let args = Args::parse();

	// Let's go!
	info!("Netron Desktop BIOS");

	{
		let mut hw = HARDWARE.lock().unwrap();
		*hw = Some(Hardware {
			boot_time: std::time::Instant::now(),
			disk_file: args
				.disk
				.map(|path| std::fs::File::open(path).expect("open disk file")),
		});
	}

	let white_on_black = common::video::Attr::new(
		common::video::TextForegroundColour::WHITE,
		common::video::TextBackgroundColour::BLACK,
		false,
	);
	for char_idx in 0..(80 * 60) {
		// Blank
		FRAMEBUFFER.write_at(char_idx * 2, b' ');
		// White on Black
		FRAMEBUFFER.write_at((char_idx * 2) + 1, white_on_black.as_u8());
	}

	// Process args
	let mut lib = None;
	for arg in std::env::args() {
		if let Some(os_path) = arg.strip_prefix("--os=") {
			info!("Loading OS from {:?}", os_path);
			lib = unsafe { Some(libloading::Library::new(os_path).expect("library to load")) };
			println!("Loaded!");
		}
	}
	let lib = lib.expect("Fetching --os=filename from args");

	// Make a window
	let mut engine = Engine::builder()
		.dimensions(640, 480)
		.title("Neotron Desktop BIOS")
		.show_frame_rate()
		.target_frame_rate(60)
		.build()
		.unwrap();
	let (sender, receiver) = std::sync::mpsc::channel();
	let mut app = MyApp {
		mode: unsafe { common::video::Mode::from_u8(0) },
		font8x16: Vec::new(),
		font8x8: Vec::new(),
		sender,
	};

	EV_QUEUE.lock().unwrap().replace(receiver);

	// Run the OS
	std::thread::spawn(move || unsafe {
		// Wait for Started message
		let queue = EV_QUEUE.lock().unwrap();
		let ev = queue.as_ref().unwrap().recv().unwrap();
		assert_eq!(ev, AppEvent::Started);
		drop(queue);
		info!("Video init complete. OS starting...");
		let main_func: libloading::Symbol<unsafe extern "C" fn(api: &'static common::Api) -> !> =
			lib.get(b"os_main").expect("os_main() not found");
		main_func(&BIOS_API);
	});

	engine.run(&mut app).unwrap();
}

/// Returns the version number of the BIOS API.
extern "C" fn api_version_get() -> common::Version {
	debug!("api_version_get()");
	common::API_VERSION
}

/// Returns a pointer to a static string slice containing the BIOS Version.
///
/// This string contains the version number and build string of the BIOS.
/// For C compatibility this string is null-terminated and guaranteed to
/// only contain ASCII characters (bytes with a value 127 or lower). We
/// also pass the length (excluding the null) to make it easy to construct
/// a Rust string. It is unspecified as to whether the string is located
/// in Flash ROM or RAM (but it's likely to be Flash ROM).
extern "C" fn bios_version_get() -> common::FfiString<'static> {
	debug!("bios_version_get()");
	common::FfiString::new("Neotron Desktop BIOS\0")
}

/// Get information about the Serial ports in the system.
///
/// Serial ports are ordered octet-oriented pipes. You can push octets
/// into them using a 'write' call, and pull bytes out of them using a
/// 'read' call. They have options which allow them to be configured at
/// different speeds, or with different transmission settings (parity
/// bits, stop bits, etc) - you set these with a call to
/// `SerialConfigure`. They may physically be a MIDI interface, an RS-232
/// port or a USB-Serial port. There is no sense of 'open' or 'close' -
/// that is an Operating System level design feature. These APIs just
/// reflect the raw hardware, in a similar manner to the registers exposed
/// by a memory-mapped UART peripheral.
extern "C" fn serial_get_info(_device: u8) -> common::FfiOption<common::serial::DeviceInfo> {
	debug!("serial_get_info()");
	common::FfiOption::None
}

/// Set the options for a given serial device. An error is returned if the
/// options are invalid for that serial device.
extern "C" fn serial_configure(
	_device: u8,
	_config: common::serial::Config,
) -> common::ApiResult<()> {
	debug!("serial_configure()");
	Err(common::Error::Unimplemented).into()
}

/// Write bytes to a serial port. There is no sense of 'opening' or
/// 'closing' the device - serial devices are always open. If the return
/// value is `Ok(n)`, the value `n` may be less than the size of the given
/// buffer. If so, that means not all of the data could be transmitted -
/// only the first `n` bytes were.
extern "C" fn serial_write(
	_device: u8,
	_data: common::FfiByteSlice,
	_timeout: common::FfiOption<common::Timeout>,
) -> common::ApiResult<usize> {
	debug!("serial_write()");
	Err(common::Error::Unimplemented).into()
}

/// Read bytes from a serial port. There is no sense of 'opening' or
/// 'closing' the device - serial devices are always open. If the return value
///  is `Ok(n)`, the value `n` may be less than the size of the given buffer.
///  If so, that means not all of the data could be received - only the
///  first `n` bytes were filled in.
extern "C" fn serial_read(
	_device: u8,
	_data: common::FfiBuffer,
	_timeout: common::FfiOption<common::Timeout>,
) -> common::ApiResult<usize> {
	debug!("serial_read()");
	Err(common::Error::Unimplemented).into()
}

/// Get the current wall time.
///
/// The Neotron BIOS does not understand time zones, leap-seconds or the
/// Gregorian calendar. It simply stores time as an incrementing number of
/// seconds since some epoch, and the number of milliseconds since that second
/// began. A day is assumed to be exactly 86,400 seconds long. This is a lot
/// like POSIX time, except we have a different epoch
/// - the Neotron epoch is 2000-01-01T00:00:00Z. It is highly recommend that you
/// store UTC in the BIOS and use the OS to handle time-zones.
///
/// If the BIOS does not have a battery-backed clock, or if that battery has
/// failed to keep time, the system starts up assuming it is the epoch.
extern "C" fn time_clock_get() -> common::Time {
	debug!("time_clock_get()");
	// 946684800 seconds between 2000-01-01 and 1970-01-01
	let epoch = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(946684800);
	let difference = epoch.elapsed().unwrap_or_default();
	// We're good until 2068, when I shall be retired.
	assert!(difference.as_secs() <= u64::from(u32::MAX));
	common::Time {
		secs: difference.as_secs() as u32,
		nsecs: difference.subsec_nanos(),
	}
}

/// Set the current wall time.
///
/// See `time_get` for a description of now the Neotron BIOS should handle
/// time.
///
/// You only need to call this whenever you get a new sense of the current
/// time (e.g. the user has updated the current time, or if you get a GPS
/// fix). The BIOS should push the time out to the battery-backed Real
/// Time Clock, if it has one.
extern "C" fn time_clock_set(time: common::Time) {
	debug!("time_clock_set({:?})", time);
}

/// Get the configuration data block.
///
/// Configuration data is, to the BIOS, just a block of bytes of a given
/// length. How it stores them is up to the BIOS - it could be EEPROM, or
/// battery-backed SRAM.
extern "C" fn configuration_get(_buffer: common::FfiBuffer) -> common::ApiResult<usize> {
	debug!("configuration_get()");
	Err(common::Error::Unimplemented).into()
}

/// Set the configuration data block.
///
/// See `configuration_get`.
extern "C" fn configuration_set(_buffer: common::FfiByteSlice) -> common::ApiResult<()> {
	debug!("configuration_set()");
	Err(common::Error::Unimplemented).into()
}

/// Does this Neotron BIOS support this video mode?
extern "C" fn video_is_valid_mode(mode: common::video::Mode) -> bool {
	debug!("video_is_valid_mode()");
	mode == common::video::Mode::new(
		common::video::Timing::T640x480,
		common::video::Format::Text8x16,
	)
}

/// Switch to a new video mode.
///
/// The contents of the screen are undefined after a call to this function.
///
/// If the BIOS does not have enough reserved RAM (or dedicated VRAM) to
/// support this mode, the change will succeed but a subsequent call to
/// `video_get_framebuffer` will return `null`. You must then supply a
/// pointer to a block of size `Mode::frame_size_bytes()` to
/// `video_set_framebuffer` before any video will appear.
extern "C" fn video_set_mode(mode: common::video::Mode) -> common::ApiResult<()> {
	info!("video_set_mode({:?})", mode);
	match mode.timing() {
		common::video::Timing::T640x480 => {
			// OK
		}
		common::video::Timing::T640x400 => {
			// OK
		}
		_ => {
			return common::ApiResult::Err(common::Error::UnsupportedConfiguration(
				mode.as_u8() as u16
			));
		}
	}
	match mode.format() {
		common::video::Format::Text8x16 => {
			// OK
		}
		common::video::Format::Text8x8 => {
			// OK
		}
		_ => {
			return common::ApiResult::Err(common::Error::UnsupportedConfiguration(
				mode.as_u8() as u16
			));
		}
	}

	// We know this is a valid video mode because it was set with `video_set_mode`.
	let mode_value = mode.as_u8();
	VIDEO_MODE.store(mode_value, Ordering::SeqCst);
	common::ApiResult::Ok(())
}

/// Returns the video mode the BIOS is currently in.
///
/// The OS should call this function immediately after start-up and note
/// the value - this is the `default` video mode which can always be
/// serviced without supplying extra RAM.
extern "C" fn video_get_mode() -> common::video::Mode {
	debug!("video_get_mode()");
	let mode_value = VIDEO_MODE.load(Ordering::SeqCst);
	// We know this is a valid video mode because it was set with `video_set_mode`.
	unsafe { common::video::Mode::from_u8(mode_value) }
}

/// Get the framebuffer address.
///
/// We can write through this address to the video framebuffer. The
/// meaning of the data we write, and the size of the region we are
/// allowed to write to, is a function of the current video mode (see
/// `video_get_mode`).
///
/// This function will return `null` if the BIOS isn't able to support the
/// current video mode from its memory reserves. If that happens, you will
/// need to use some OS RAM or Application RAM and provide that as a
/// framebuffer to `video_set_framebuffer`. The BIOS will always be able
/// to provide the 'basic' text buffer experience from reserves, so this
/// function will never return `null` on start-up.
extern "C" fn video_get_framebuffer() -> *mut u8 {
	let p = FRAMEBUFFER.get_pointer();
	debug!("video_get_framebuffer() -> {:p}", p);
	p
}

/// Set the framebuffer address.
///
/// Tell the BIOS where it should start fetching pixel or textual data from
/// (depending on the current video mode).
///
/// This value is forgotten after a video mode change and must be re-supplied.
///
/// # Safety
///
/// The pointer must point to enough video memory to handle the current video
/// mode, and any future video mode you set.
unsafe extern "C" fn video_set_framebuffer(_buffer: *const u8) -> common::ApiResult<()> {
	Err(common::Error::Unimplemented).into()
}

/// Find out whether the given video mode needs more VRAM than we currently have.
///
/// The answer is no for any currently supported video mode (which is just the four text modes right now).
extern "C" fn video_mode_needs_vram(_mode: common::video::Mode) -> bool {
	debug!("video_mode_needs_vram()");
	false
}

/// Find out how large a given region of memory is.
///
/// The first region is the 'main application region' and is defined to always
/// start at address `0x2000_0000` on a standard Cortex-M system. This
/// application region stops just before the BIOS reserved memory, at the top of
/// the internal SRAM. The OS will have been linked to use the first 1 KiB of
/// this region.
///
/// Other regions may be located at other addresses (e.g. external DRAM or
/// PSRAM).
///
/// The OS will always load non-relocatable applications into the bottom of
/// Region 0. It can allocate OS specific structures from any other Region (if
/// any), or from the top of Region 0 (although this reduces the maximum
/// application space available). The OS will prefer lower numbered regions
/// (other than Region 0), so faster memory should be listed first.
///
/// If the region number given is invalid, the function returns `(null, 0)`.
extern "C" fn memory_get_region(region: u8) -> common::FfiOption<common::MemoryRegion> {
	static mut MEMORY_BLOCK: (*mut u8, usize) = (std::ptr::null_mut(), 0);
	match region {
		0 => {
			if unsafe { MEMORY_BLOCK.0.is_null() } {
				// Allocate 256 KiB of storage space for the OS to use
				let mut data = Box::new([0u8; 256 * 1024]);
				unsafe {
					MEMORY_BLOCK.0 = data.as_mut_ptr() as *mut u8;
					MEMORY_BLOCK.1 = std::mem::size_of_val(&*data);
				}
				std::mem::forget(data);
			}
			common::FfiOption::Some(common::MemoryRegion {
				start: unsafe { MEMORY_BLOCK.0 },
				length: unsafe { MEMORY_BLOCK.1 },
				kind: common::MemoryKind::Ram,
			})
		}
		_ => common::FfiOption::None,
	}
}

/// Get the next available HID event, if any.
///
/// This function doesn't block. It will return `Ok(None)` if there is no event ready.
extern "C" fn hid_get_event() -> common::ApiResult<common::FfiOption<common::hid::HidEvent>> {
	let queue = EV_QUEUE.lock().unwrap();
	match queue.as_ref().unwrap().try_recv() {
		Ok(AppEvent::KeyUp(key)) => {
			let code = common::hid::HidEvent::KeyRelease(convert_keycode(key));
			debug!("hid_get_event() -> {:?}", code);
			common::ApiResult::Ok(common::FfiOption::Some(code))
		}
		Ok(AppEvent::KeyDown(key)) => {
			let code = common::hid::HidEvent::KeyPress(convert_keycode(key));
			debug!("hid_get_event() -> {:?}", code);
			common::ApiResult::Ok(common::FfiOption::Some(code))
		}
		_ => common::ApiResult::Ok(common::FfiOption::None),
	}
}

/// Convert a pix-engine keycode into a Neotron BIOS keycode
fn convert_keycode(key: Key) -> common::hid::KeyCode {
	match key {
		Key::Backspace => common::hid::KeyCode::Backspace,
		Key::Tab => common::hid::KeyCode::Tab,
		Key::Return => common::hid::KeyCode::Return,
		Key::Escape => common::hid::KeyCode::Escape,
		Key::Space => common::hid::KeyCode::Spacebar,
		// Key::Exclaim => common::hid::KeyCode::Exclaim,
		// Key::Quotedbl => common::hid::KeyCode::Quotedbl,
		Key::Hash => common::hid::KeyCode::Oem7,
		// Key::Dollar => common::hid::KeyCode::Dollar,
		// Key::Percent => common::hid::KeyCode::Percent,
		// Key::Ampersand => common::hid::KeyCode::Ampersand,
		Key::Quote => common::hid::KeyCode::Oem3,
		// Key::LeftParen => common::hid::KeyCode::LeftParen,
		// Key::RightParen => common::hid::KeyCode::RightParen,
		// Key::Asterisk => common::hid::KeyCode::Asterisk,
		// Key::Plus => common::hid::KeyCode::Plus,
		Key::Comma => common::hid::KeyCode::OemComma,
		Key::Minus => common::hid::KeyCode::OemMinus,
		Key::Period => common::hid::KeyCode::OemPeriod,
		Key::Slash => common::hid::KeyCode::Oem2,
		Key::Num0 => common::hid::KeyCode::Key0,
		Key::Num1 => common::hid::KeyCode::Key1,
		Key::Num2 => common::hid::KeyCode::Key2,
		Key::Num3 => common::hid::KeyCode::Key3,
		Key::Num4 => common::hid::KeyCode::Key4,
		Key::Num5 => common::hid::KeyCode::Key5,
		Key::Num6 => common::hid::KeyCode::Key6,
		Key::Num7 => common::hid::KeyCode::Key7,
		Key::Num8 => common::hid::KeyCode::Key8,
		Key::Num9 => common::hid::KeyCode::Key9,
		// Key::Colon => common::hid::KeyCode::Colon,
		Key::Semicolon => common::hid::KeyCode::Oem1,
		// Key::Less => common::hid::KeyCode::Less,
		Key::Equals => common::hid::KeyCode::OemPlus,
		// Key::Greater => common::hid::KeyCode::Greater,
		// Key::Question => common::hid::KeyCode::Question,
		// Key::At => common::hid::KeyCode::At,
		Key::LeftBracket => common::hid::KeyCode::Oem4,
		Key::Backslash => common::hid::KeyCode::Oem5,
		Key::RightBracket => common::hid::KeyCode::Oem6,
		// Key::Caret => common::hid::KeyCode::Caret,
		// Key::Underscore => common::hid::KeyCode::Underscore,
		Key::Backquote => common::hid::KeyCode::Oem8,
		Key::A => common::hid::KeyCode::A,
		Key::B => common::hid::KeyCode::B,
		Key::C => common::hid::KeyCode::C,
		Key::D => common::hid::KeyCode::D,
		Key::E => common::hid::KeyCode::E,
		Key::F => common::hid::KeyCode::F,
		Key::G => common::hid::KeyCode::G,
		Key::H => common::hid::KeyCode::H,
		Key::I => common::hid::KeyCode::I,
		Key::J => common::hid::KeyCode::J,
		Key::K => common::hid::KeyCode::K,
		Key::L => common::hid::KeyCode::L,
		Key::M => common::hid::KeyCode::M,
		Key::N => common::hid::KeyCode::N,
		Key::O => common::hid::KeyCode::O,
		Key::P => common::hid::KeyCode::P,
		Key::Q => common::hid::KeyCode::Q,
		Key::R => common::hid::KeyCode::R,
		Key::S => common::hid::KeyCode::S,
		Key::T => common::hid::KeyCode::T,
		Key::U => common::hid::KeyCode::U,
		Key::V => common::hid::KeyCode::V,
		Key::W => common::hid::KeyCode::W,
		Key::X => common::hid::KeyCode::X,
		Key::Y => common::hid::KeyCode::Y,
		Key::Z => common::hid::KeyCode::Z,
		Key::Delete => common::hid::KeyCode::Delete,
		Key::CapsLock => common::hid::KeyCode::CapsLock,
		Key::F1 => common::hid::KeyCode::F1,
		Key::F2 => common::hid::KeyCode::F2,
		Key::F3 => common::hid::KeyCode::F3,
		Key::F4 => common::hid::KeyCode::F4,
		Key::F5 => common::hid::KeyCode::F5,
		Key::F6 => common::hid::KeyCode::F6,
		Key::F7 => common::hid::KeyCode::F7,
		Key::F8 => common::hid::KeyCode::F8,
		Key::F9 => common::hid::KeyCode::F9,
		Key::F10 => common::hid::KeyCode::F10,
		Key::F11 => common::hid::KeyCode::F11,
		Key::F12 => common::hid::KeyCode::F12,
		Key::PrintScreen => common::hid::KeyCode::PrintScreen,
		Key::ScrollLock => common::hid::KeyCode::ScrollLock,
		Key::Pause => common::hid::KeyCode::PauseBreak,
		Key::Insert => common::hid::KeyCode::Insert,
		Key::Home => common::hid::KeyCode::Home,
		Key::PageUp => common::hid::KeyCode::PageUp,
		Key::End => common::hid::KeyCode::End,
		Key::PageDown => common::hid::KeyCode::PageDown,
		Key::Right => common::hid::KeyCode::ArrowRight,
		Key::Left => common::hid::KeyCode::ArrowLeft,
		Key::Down => common::hid::KeyCode::ArrowDown,
		Key::Up => common::hid::KeyCode::ArrowUp,
		Key::NumLock => common::hid::KeyCode::NumpadLock,
		Key::KpDivide => common::hid::KeyCode::NumpadDivide,
		Key::KpMultiply => common::hid::KeyCode::NumpadMultiply,
		Key::KpMinus => common::hid::KeyCode::NumpadSubtract,
		Key::KpPlus => common::hid::KeyCode::NumpadAdd,
		Key::KpEnter => common::hid::KeyCode::NumpadEnter,
		Key::Kp1 => common::hid::KeyCode::Numpad1,
		Key::Kp2 => common::hid::KeyCode::Numpad2,
		Key::Kp3 => common::hid::KeyCode::Numpad3,
		Key::Kp4 => common::hid::KeyCode::Numpad4,
		Key::Kp5 => common::hid::KeyCode::Numpad5,
		Key::Kp6 => common::hid::KeyCode::Numpad6,
		Key::Kp7 => common::hid::KeyCode::Numpad7,
		Key::Kp8 => common::hid::KeyCode::Numpad8,
		Key::Kp9 => common::hid::KeyCode::Numpad9,
		Key::Kp0 => common::hid::KeyCode::Numpad0,
		Key::KpPeriod => common::hid::KeyCode::NumpadPeriod,
		// Key::KpEquals => common::hid::KeyCode::KpEquals,
		// Key::KpComma => common::hid::KeyCode::KpComma,
		Key::LCtrl => common::hid::KeyCode::LControl,
		Key::LShift => common::hid::KeyCode::LShift,
		Key::LAlt => common::hid::KeyCode::LAlt,
		Key::LGui => common::hid::KeyCode::LWin,
		Key::RCtrl => common::hid::KeyCode::RControl,
		Key::RShift => common::hid::KeyCode::RShift,
		Key::RAlt => common::hid::KeyCode::RAltGr,
		Key::RGui => common::hid::KeyCode::RWin,
		_ => common::hid::KeyCode::X,
	}
}

/// Control the keyboard LEDs.
extern "C" fn hid_set_leds(_leds: common::hid::KeyboardLeds) -> common::ApiResult<()> {
	debug!("hid_set_leds()");
	Err(common::Error::Unimplemented).into()
}

/// Wait for the next occurence of the specified video scan-line.
///
/// In general we must assume that the video memory is read top-to-bottom
/// as the picture is being drawn on the monitor (e.g. via a VGA video
/// signal). If you modify video memory during this *drawing period*
/// there is a risk that the image on the monitor (however briefly) may
/// contain some parts from before the modification and some parts from
/// after. This can given rise to the *tearing effect* where it looks
/// like the screen has been torn (or ripped) across because there is a
/// discontinuity part-way through the image.
///
/// This function busy-waits until the video drawing has reached a
/// specified scan-line on the video frame.
///
/// There is no error code here. If the line you ask for is beyond the
/// number of visible scan-lines in the current video mode, it waits util
/// the last visible scan-line is complete.
///
/// If you wait for the last visible line until drawing, you stand the
/// best chance of your pixels operations on the video RAM being
/// completed before scan-lines start being sent to the monitor for the
/// next frame.
///
/// You can also use this for a crude `16.7 ms` delay but note that
/// some video modes run at `70 Hz` and so this would then give you a
/// `14.3ms` second delay.
extern "C" fn video_wait_for_line(_line: u16) {
	debug!("video_wait_for_line()");
	// TODO
}

extern "C" fn video_get_palette(index: u8) -> common::FfiOption<common::video::RGBColour> {
	debug!("video_get_palette({})", index);
	let entry = PALETTE.get(usize::from(index));
	let entry_value =
		entry.map(|raw| common::video::RGBColour::from_packed(raw.load(Ordering::SeqCst)));
	match entry_value {
		Some(rgb) => common::FfiOption::Some(rgb),
		None => common::FfiOption::None,
	}
}

extern "C" fn video_set_palette(index: u8, rgb: common::video::RGBColour) {
	debug!("video_set_palette({}, #{:6x})", index, rgb.as_packed());
	if let Some(e) = PALETTE.get(usize::from(index)) {
		e.store(rgb.as_packed(), Ordering::SeqCst);
	}
}

unsafe extern "C" fn video_set_whole_palette(
	palette: *const common::video::RGBColour,
	length: usize,
) {
	debug!("video_set_whole_palette({:p}, {})", palette, length);
	let slice = std::slice::from_raw_parts(palette, length);
	for (entry, new_rgb) in PALETTE.iter().zip(slice) {
		entry.store(new_rgb.as_packed(), Ordering::SeqCst);
	}
}

extern "C" fn i2c_bus_get_info(_i2c_bus: u8) -> common::FfiOption<common::i2c::BusInfo> {
	debug!("i2c_bus_get_info");
	common::FfiOption::None
}

extern "C" fn i2c_write_read(
	_i2c_bus: u8,
	_i2c_device_address: u8,
	_tx: common::FfiByteSlice,
	_tx2: common::FfiByteSlice,
	_rx: common::FfiBuffer,
) -> common::ApiResult<()> {
	debug!("i2c_write_read");
	common::ApiResult::Err(common::Error::Unimplemented)
}

extern "C" fn audio_mixer_channel_get_info(
	_audio_mixer_id: u8,
) -> common::FfiOption<common::audio::MixerChannelInfo> {
	debug!("audio_mixer_channel_get_info");
	common::FfiOption::None
}

extern "C" fn audio_mixer_channel_set_level(
	_audio_mixer_id: u8,
	_level: u8,
) -> common::ApiResult<()> {
	debug!("audio_mixer_channel_set_level");
	common::ApiResult::Err(common::Error::Unimplemented)
}

extern "C" fn audio_output_set_config(_config: common::audio::Config) -> common::ApiResult<()> {
	debug!("audio_output_set_config");
	common::ApiResult::Err(common::Error::Unimplemented)
}

extern "C" fn audio_output_get_config() -> common::ApiResult<common::audio::Config> {
	debug!("audio_output_get_config");
	common::ApiResult::Err(common::Error::Unimplemented)
}

unsafe extern "C" fn audio_output_data(_samples: common::FfiByteSlice) -> common::ApiResult<usize> {
	debug!("audio_output_data");
	common::ApiResult::Err(common::Error::Unimplemented)
}

extern "C" fn audio_output_get_space() -> common::ApiResult<usize> {
	debug!("audio_output_get_space");
	common::ApiResult::Err(common::Error::Unimplemented)
}

extern "C" fn audio_input_set_config(_config: common::audio::Config) -> common::ApiResult<()> {
	debug!("audio_input_set_config");
	common::ApiResult::Err(common::Error::Unimplemented)
}

extern "C" fn audio_input_get_config() -> common::ApiResult<common::audio::Config> {
	debug!("audio_input_get_config");
	common::ApiResult::Err(common::Error::Unimplemented)
}

extern "C" fn audio_input_data(_samples: common::FfiBuffer) -> common::ApiResult<usize> {
	debug!("audio_input_data");
	common::ApiResult::Err(common::Error::Unimplemented)
}

extern "C" fn audio_input_get_count() -> common::ApiResult<usize> {
	debug!("audio_input_get_count");
	common::ApiResult::Err(common::Error::Unimplemented)
}

extern "C" fn bus_select(_periperal_id: common::FfiOption<u8>) {
	debug!("bus_select");
}

extern "C" fn bus_get_info(_periperal_id: u8) -> common::FfiOption<common::bus::PeripheralInfo> {
	debug!("bus_get_info");
	common::FfiOption::None
}

extern "C" fn bus_write_read(
	_tx: common::FfiByteSlice,
	_tx2: common::FfiByteSlice,
	_rx: common::FfiBuffer,
) -> common::ApiResult<()> {
	debug!("bus_write_read");
	common::ApiResult::Err(common::Error::Unimplemented)
}

extern "C" fn bus_exchange(_buffer: common::FfiBuffer) -> common::ApiResult<()> {
	debug!("bus_exchange");
	common::ApiResult::Err(common::Error::Unimplemented)
}

extern "C" fn time_ticks_get() -> common::Ticks {
	let mut hw_guard = HARDWARE.lock().unwrap();
	let hw = hw_guard.as_mut().unwrap();
	let boot_time = hw.boot_time;
	let difference = boot_time.elapsed();
	debug!("time_ticks_get() -> {}", difference.as_millis());
	common::Ticks(difference.as_millis() as u64)
}

/// We simulate a 1 kHz tick
extern "C" fn time_ticks_per_second() -> common::Ticks {
	debug!("time_ticks_per_second()");
	common::Ticks(1000)
}

extern "C" fn bus_interrupt_status() -> u32 {
	debug!("bus_interrupt_status()");
	0
}

extern "C" fn block_dev_get_info(dev_id: u8) -> common::FfiOption<common::block_dev::DeviceInfo> {
	debug!("block_dev_get_info(dev_id: {})", dev_id);
	let mut hw_guard = HARDWARE.lock().unwrap();
	let hw = hw_guard.as_mut().unwrap();
	if dev_id == 0 {
		match &mut hw.disk_file {
			Some(file) => common::FfiOption::Some(common::block_dev::DeviceInfo {
				name: common::FfiString::new("File0"),
				device_type: common::block_dev::DeviceType::HardDiskDrive,
				block_size: BLOCK_SIZE as u32,
				num_blocks: file.metadata().unwrap().len() / (BLOCK_SIZE as u64),
				ejectable: false,
				removable: false,
				media_present: true,
				read_only: false,
			}),
			None => common::FfiOption::None,
		}
	} else {
		common::FfiOption::None
	}
}

extern "C" fn block_dev_eject(dev_id: u8) -> common::ApiResult<()> {
	debug!("block_dev_eject(dev_id: {})", dev_id);
	common::ApiResult::Ok(())
}

extern "C" fn block_write(
	dev_id: u8,
	block_idx: common::block_dev::BlockIdx,
	num_blocks: u8,
	buffer: common::FfiByteSlice,
) -> common::ApiResult<()> {
	debug!(
		"block_write(dev_id: {}, block_id: {}, num_blocks: {}, buffer_len: {})",
		dev_id, block_idx.0, num_blocks, buffer.data_len
	);
	let mut hw_guard = HARDWARE.lock().unwrap();
	let hw = hw_guard.as_mut().unwrap();
	if dev_id == 0 {
		match &mut hw.disk_file {
			Some(file) => {
				if file
					.seek(std::io::SeekFrom::Start(block_idx.0 * BLOCK_SIZE as u64))
					.is_err()
				{
					return common::ApiResult::Err(common::Error::BlockOutOfBounds);
				}
				let buffer_slice = &buffer.as_slice()[0..usize::from(num_blocks) * BLOCK_SIZE];
				if let Err(e) = file.write_all(buffer_slice) {
					log::warn!("Failed to write to disk image: {:?}", e);
					return common::ApiResult::Err(common::Error::DeviceError(0));
				}
				common::ApiResult::Ok(())
			}
			None => common::ApiResult::Err(common::Error::DeviceError(0)),
		}
	} else {
		common::ApiResult::Err(common::Error::InvalidDevice)
	}
}

extern "C" fn block_read(
	dev_id: u8,
	block_idx: common::block_dev::BlockIdx,
	num_blocks: u8,
	mut buffer: common::FfiBuffer,
) -> common::ApiResult<()> {
	debug!(
		"block_read(dev_id: {}, block_id: {}, num_blocks: {}, buffer_len: {})",
		dev_id, block_idx.0, num_blocks, buffer.data_len
	);
	let mut hw_guard = HARDWARE.lock().unwrap();
	let hw = hw_guard.as_mut().unwrap();
	if dev_id == 0 {
		match &mut hw.disk_file {
			Some(file) => {
				if file
					.seek(std::io::SeekFrom::Start(block_idx.0 * BLOCK_SIZE as u64))
					.is_err()
				{
					return common::ApiResult::Err(common::Error::BlockOutOfBounds);
				}
				if let Some(buffer_slice) = buffer.as_mut_slice() {
					let buffer_slice = &mut buffer_slice[0..usize::from(num_blocks) * BLOCK_SIZE];
					if let Err(e) = file.read_exact(buffer_slice) {
						log::warn!("Failed to read from disk image: {:?}", e);
						return common::ApiResult::Err(common::Error::DeviceError(0));
					}
				}
				common::ApiResult::Ok(())
			}
			None => common::ApiResult::Err(common::Error::DeviceError(0)),
		}
	} else {
		common::ApiResult::Err(common::Error::InvalidDevice)
	}
}

extern "C" fn block_verify(
	dev_id: u8,
	block_idx: common::block_dev::BlockIdx,
	num_blocks: u8,
	buffer: common::FfiByteSlice,
) -> common::ApiResult<()> {
	debug!(
		"block_read(dev_id: {}, block_id: {}, num_blocks: {}, buffer_len: {})",
		dev_id, block_idx.0, num_blocks, buffer.data_len
	);
	let mut hw_guard = HARDWARE.lock().unwrap();
	let hw = hw_guard.as_mut().unwrap();
	if dev_id == 0 {
		match &mut hw.disk_file {
			Some(file) => {
				if file
					.seek(std::io::SeekFrom::Start(block_idx.0 * BLOCK_SIZE as u64))
					.is_err()
				{
					return common::ApiResult::Err(common::Error::BlockOutOfBounds);
				}
				let buffer_slice = &buffer.as_slice()[0..usize::from(num_blocks) * BLOCK_SIZE];
				let mut read_buffer = vec![0u8; buffer_slice.len()];
				if let Err(e) = file.read_exact(&mut read_buffer) {
					log::warn!("Failed to write to disk image: {:?}", e);
					return common::ApiResult::Err(common::Error::DeviceError(0));
				}
				if read_buffer.as_slice() == buffer_slice {
					common::ApiResult::Ok(())
				} else {
					common::ApiResult::Err(common::Error::DeviceError(1))
				}
			}
			None => common::ApiResult::Err(common::Error::DeviceError(0)),
		}
	} else {
		common::ApiResult::Err(common::Error::InvalidDevice)
	}
}

extern "C" fn power_idle() {
	std::thread::sleep(std::time::Duration::from_millis(1));
}

extern "C" fn power_control(mode: common::PowerMode) -> ! {
	println!("Got power mode {:?}, but quitting...", mode);
	std::process::exit(0);
}

extern "C" fn compare_and_swap_bool(
	item: &std::sync::atomic::AtomicBool,
	old_value: bool,
	new_value: bool,
) -> bool {
	item.compare_exchange(old_value, new_value, Ordering::SeqCst, Ordering::SeqCst)
		.is_ok()
}

// ===========================================================================
// Impl Blocks
// ===========================================================================

impl MyApp {
	const NUM_FG: usize = 16;

	/// Generate an RGBA texture for each glyph, in each foreground colour.
	///
	/// We have 256 glyphs, in each of 16 colours, so this is expensive and
	/// slow. But it makes rendering text acceptably fast.
	fn render_font(
		font: &font::Font,
		texture_buffer: &mut Vec<TextureId>,
		s: &mut PixState,
	) -> PixResult<()> {
		let mut slot = 0;
		for glyph in 0..=255 {
			for palette_entry in PALETTE.iter().take(Self::NUM_FG) {
				let fg = RGBColour::from_packed(palette_entry.load(Ordering::Relaxed));
				info!("Drawing {glyph} in {:06x}", fg.as_packed());
				let texture_id = if texture_buffer.len() > slot {
					texture_buffer[slot]
				} else {
					let id = s.create_texture(8, font.height as u32, PixelFormat::Rgba)?;
					texture_buffer.push(id);
					id
				};
				slot += 1;
				s.set_texture_target(texture_id)?;
				s.background(Color::TRANSPARENT);
				s.clear()?;
				s.stroke(rgb!(fg.red(), fg.green(), fg.blue(), 255));
				for font_y in 0..(font.height as i32) {
					let mut font_line =
						font.data[((glyph as usize) * font.height) + font_y as usize];
					for font_x in 0..8i32 {
						if (font_line & 0x80) != 0 {
							s.point(Point::new([font_x, font_y]))?;
						};
						font_line <<= 1;
					}
				}
				s.clear_texture_target();
			}
		}
		Ok(())
	}

	/// Generate an RGBA texture for each glyph, in each foreground colour, in
	/// each font.
	fn render_glyphs(&mut self, s: &mut PixState) -> PixResult<()> {
		Self::render_font(&font::font16::FONT, &mut self.font8x16, s)?;
		Self::render_font(&font::font8::FONT, &mut self.font8x8, s)?;
		Ok(())
	}
}

impl PixEngine for MyApp {
	/// Perform application initialisation.
	fn on_start(&mut self, s: &mut PixState) -> PixResult<()> {
		self.render_glyphs(s)?;
		// Let the rest of the OS start now
		self.sender.send(AppEvent::Started).unwrap();
		Ok(())
	}

	/// Terminate the process to ensure the OS thread dies too.
	fn on_stop(&mut self, _s: &mut PixState) -> PixResult<()> {
		std::process::exit(0);
	}

	/// Called whenever the app has an event to process.
	///
	/// We send key up and key down events into a queue for the OS to process later.
	fn on_event(&mut self, _s: &mut PixState, event: &Event) -> PixResult<bool> {
		match event {
			Event::KeyUp {
				key: Some(key),
				keymod: _,
				repeat: _,
			} => {
				self.sender.send(AppEvent::KeyUp(*key)).unwrap();
				Ok(true)
			}
			Event::KeyDown {
				key: Some(key),
				keymod: _,
				repeat: _,
			} => {
				self.sender.send(AppEvent::KeyDown(*key)).unwrap();
				Ok(true)
			}
			_ => Ok(false),
		}
	}

	/// Called in a tight-loop to update the application.
	///
	/// We convert the contents of `FRAMEBUFFER` into pixels on the canvas.
	fn on_update(&mut self, s: &mut PixState) -> PixResult<()> {
		let mode_value = VIDEO_MODE.load(Ordering::SeqCst);
		let new_mode = unsafe { common::video::Mode::from_u8(mode_value) };
		if new_mode != self.mode {
			self.mode = new_mode;
			let width = (new_mode.horizontal_pixels() as f32) * SCALE_FACTOR;
			let height = (new_mode.vertical_lines() as f32) * SCALE_FACTOR;
			info!("Window set to {} x {}", width, height);
			s.set_window_dimensions((width as u32, height as u32))?;
			s.scale(SCALE_FACTOR, SCALE_FACTOR)?;
		}

		s.blend_mode(BlendMode::Blend);

		let (font, font_height) = match self.mode.format() {
			common::video::Format::Text8x16 => (&self.font8x16, 16),
			common::video::Format::Text8x8 => (&self.font8x8, 8),
			_ => {
				// Unknown mode - do nothing
				return Ok(());
			}
		};

		let num_cols = self.mode.text_width().unwrap();
		let num_rows = self.mode.text_height().unwrap();
		// FRAMEBUFFER is an num_cols x num_rows size array of (u8_glyph, u8_attr).
		for row in 0..num_rows {
			let y = row * font_height;
			for col in 0..num_cols {
				let cell_no = (row * num_cols) + col;
				let byte_offset = usize::from(cell_no) * 2;
				let x = col * 8;
				let glyph = FRAMEBUFFER.get_at(byte_offset);
				let attr = common::video::Attr(FRAMEBUFFER.get_at(byte_offset + 1));
				let fg_idx = attr.fg().as_u8();
				let bg_idx = attr.bg().as_u8();
				let bg =
					RGBColour::from_packed(PALETTE[usize::from(bg_idx)].load(Ordering::SeqCst));
				let glyph_box = rect!(i32::from(x), i32::from(y), 8i32, font_height as i32,);
				s.fill(rgb!(bg.red(), bg.green(), bg.blue()));
				s.rect(glyph_box)?;
				let slot = (usize::from(glyph) * Self::NUM_FG) + usize::from(fg_idx);
				s.texture(font[slot], None, Some(glyph_box))?;
			}
		}

		Ok(())
	}
}

impl<const N: usize> Framebuffer<N> {
	/// Create a new blank Framebuffer.
	///
	/// Everything is zero initialised.
	const fn new() -> Framebuffer<N> {
		Framebuffer {
			contents: std::cell::UnsafeCell::new([0u8; N]),
		}
	}

	/// Set a byte in the framebuffer.
	///
	/// Panics if you try and write out of bounds.
	///
	/// Uses volatile writes.
	fn write_at(&self, offset: usize, value: u8) {
		if offset > std::mem::size_of_val(&self.contents) {
			panic!("Out of bounds framebuffer write");
		}
		unsafe {
			let array_ptr = self.contents.get() as *mut u8;
			let byte_ptr = array_ptr.add(offset);
			byte_ptr.write_volatile(value);
		}
	}

	/// Get a byte from the framebuffer.
	///
	/// Panics if you try and read out of bounds.
	///
	/// Uses volatile reads.
	fn get_at(&self, offset: usize) -> u8 {
		if offset > std::mem::size_of_val(&self.contents) {
			panic!("Out of bounds framebuffer read");
		}
		unsafe {
			let array_ptr = self.contents.get() as *const u8;
			let byte_ptr = array_ptr.add(offset);
			byte_ptr.read_volatile()
		}
	}

	/// Get a pointer to the framebuffer you can give to the OS.
	fn get_pointer(&self) -> *mut u8 {
		self.contents.get() as *mut u8
	}
}

unsafe impl<const N: usize> Sync for Framebuffer<N> {}

// ===========================================================================
// End of File
// ===========================================================================
