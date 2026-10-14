//! Linux-spezifische Teile: Launcher, Provider, Kernel-Probe, Landlock, seccomp, Mounts, Netz
//! und cgroups. Systemaufrufe ohne sichere Hülle gehen direkt über `libc`; jede Stelle trägt
//! eine `SAFETY`-Begründung.
#![allow(
    unsafe_code,
    reason = "Namespaces, Mounts, seccomp und fork gibt es nur über libc"
)]

pub mod cgroup;
pub mod landlock;
pub mod launcher;
pub mod mounts;
pub mod msg;
pub mod net;
pub mod probe;
pub mod provider;
pub mod seccomp;
pub mod util;
