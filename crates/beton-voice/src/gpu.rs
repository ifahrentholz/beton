//! GPU-Probe über die Backend-Registry von ggml (VOI-003).
//!
//! whisper.cpp wählt beim Laden selbst ein GPU-Backend und rechnet still auf der CPU weiter,
//! wenn dessen Initialisierung scheitert. Damit beton das Backend melden (`beton doctor`) und
//! einen Rückfall mit Warn-Log ausweisen kann, prüft diese Probe vorher, ob sich ein
//! GPU-Gerät der Registry initialisieren lässt, und gibt es sofort wieder frei.
#![allow(
    unsafe_code,
    reason = "die Geräteliste von ggml gibt es nur über die C-API von whisper.cpp"
)]

use std::ffi::CStr;

use whisper_rs::whisper_rs_sys as sys;

use crate::engine::Backend;

/// Ein nutzbares GPU-Gerät.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuDevice {
    pub backend: Backend,
    /// Gerätename von ggml, z. B. `MTL0` oder `Metal`.
    pub name: String,
    /// Beschreibung, z. B. `Apple M3 Pro`.
    pub description: String,
}

/// Prüft, ob ein GPU-Backend nutzbar ist. Abstrakt, damit Tests einen Fehlschlag simulieren
/// können (VOI-003 AC2).
pub trait GpuProbe: Send + Sync {
    /// `Ok(None)`: kein GPU-Backend in diesem Build bzw. kein Gerät. `Err`: ein Gerät ist da,
    /// lässt sich aber nicht initialisieren.
    fn probe(&self) -> Result<Option<GpuDevice>, String>;
}

/// Die echte Probe über ggml.
#[derive(Debug, Clone, Copy, Default)]
pub struct GgmlProbe;

fn text(ptr: *const std::os::raw::c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: ggml liefert nullterminierte, statische Zeichenketten des Geräts.
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

fn backend_for(name: &str, description: &str) -> Option<Backend> {
    let n = format!("{name} {description}").to_ascii_lowercase();
    if n.contains("mtl") || n.contains("metal") {
        Some(Backend::Metal)
    } else if n.contains("cuda") {
        Some(Backend::Cuda)
    } else {
        None
    }
}

impl GpuProbe for GgmlProbe {
    fn probe(&self) -> Result<Option<GpuDevice>, String> {
        // SAFETY: Die Registry von ggml ist threadsicher initialisiert; Index < Anzahl.
        let count = unsafe { sys::ggml_backend_dev_count() };
        for i in 0..count {
            // SAFETY: i < count.
            let dev = unsafe { sys::ggml_backend_dev_get(i) };
            if dev.is_null() {
                continue;
            }
            // SAFETY: dev ist ein gültiges Gerät der Registry.
            let kind = unsafe { sys::ggml_backend_dev_type(dev) };
            if kind != sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU
                && kind != sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU
            {
                continue;
            }
            // SAFETY: wie oben.
            let name = text(unsafe { sys::ggml_backend_dev_name(dev) });
            // SAFETY: wie oben.
            let description = text(unsafe { sys::ggml_backend_dev_description(dev) });
            let Some(backend) = backend_for(&name, &description) else {
                continue;
            };
            // SAFETY: dev ist gültig; ohne Parameter (null) initialisiert ggml mit Defaults.
            let handle = unsafe { sys::ggml_backend_dev_init(dev, std::ptr::null()) };
            if handle.is_null() {
                return Err(format!(
                    "{} ({description}) ließ sich nicht initialisieren",
                    backend.as_str()
                ));
            }
            // SAFETY: handle stammt von ggml_backend_dev_init und wird genau einmal freigegeben.
            unsafe { sys::ggml_backend_free(handle) };
            return Ok(Some(GpuDevice {
                backend,
                name,
                description,
            }));
        }
        Ok(None)
    }
}

/// Erwartet dieser Build eine GPU? (macOS mit Metal; CUDA folgt mit DIST-001.)
pub fn gpu_expected() -> bool {
    cfg!(target_os = "macos")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_device_names_to_backends() {
        assert_eq!(backend_for("MTL0", "Apple M3 Pro"), Some(Backend::Metal));
        assert_eq!(backend_for("Metal", ""), Some(Backend::Metal));
        assert_eq!(backend_for("CUDA0", "RTX"), Some(Backend::Cuda));
        assert_eq!(backend_for("Vulkan0", "x"), None);
    }

    /// VOI-003 AC1: Auf Apple Silicon findet die Probe Metal.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn voi_003_ac1_apple_silicon_probe_finds_metal() {
        let dev = GgmlProbe.probe().unwrap().expect("Metal-Gerät");
        assert_eq!(dev.backend, Backend::Metal, "{dev:?}");
    }
}
