//! seccomp-Filter der Sandbox (SBX-008): Denylist mit `EPERM`, Default „erlauben“.
//!
//! - Namespaces, Mounts, Kernel-Module, Tracing, BPF, Keyrings, io_uring, Datei-Handles, Uhr,
//!   Swap und Reboot → `EPERM`.
//! - `clone` mit einem `CLONE_NEW*`-Flag → `EPERM`; `clone3` → `ENOSYS` (die Flags liegen in
//!   einer Struktur, die seccomp nicht lesen kann; glibc fällt dann auf `clone` zurück).
//! - `socket` nur für `AF_UNIX`, `AF_INET`, `AF_INET6`.
//! - `ioctl(TIOCSTI)`/`ioctl(TIOCLINUX)` → `EPERM` (keine Eingaben in das Terminal des
//!   Benutzers schieben).
//! - Compat-ABIs: Jeder Syscall mit fremder Architektur (i386 über `int 0x80`) beendet den
//!   Prozess (Arch-Prüfung des Filters); x32-Syscalls (Bit 30 der Nummer) → `EPERM`.
//! - Landlock-ABI < 3 (Kernel < 6.2) kennt keine Truncate-Kontrolle: Dann sind `truncate(2)`
//!   und `open(…, O_RDONLY | O_TRUNC)` gesperrt, `openat2` liefert `ENOSYS`.

use std::collections::BTreeMap;

use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen as Len, SeccompCmpOp as Op, SeccompCondition,
    SeccompFilter, SeccompRule, TargetArch, sock_filter,
};

/// Unbedingt gesperrte Syscalls.
#[cfg_attr(not(target_arch = "x86_64"), allow(unused_mut))]
fn denied() -> Vec<i64> {
    let mut v = vec![
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_fsopen,
        libc::SYS_fsconfig,
        libc::SYS_fsmount,
        libc::SYS_fspick,
        libc::SYS_move_mount,
        libc::SYS_open_tree,
        libc::SYS_mount_setattr,
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_pidfd_getfd,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_userfaultfd,
        libc::SYS_kexec_load,
        libc::SYS_kexec_file_load,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
        libc::SYS_open_by_handle_at,
        libc::SYS_name_to_handle_at,
        libc::SYS_reboot,
        libc::SYS_swapon,
        libc::SYS_swapoff,
        libc::SYS_settimeofday,
        libc::SYS_clock_settime,
        libc::SYS_clock_adjtime,
        libc::SYS_adjtimex,
        libc::SYS_acct,
        libc::SYS_quotactl,
        libc::SYS_syslog,
        libc::SYS_vhangup,
        libc::SYS_sethostname,
        libc::SYS_setdomainname,
    ];
    #[cfg(target_arch = "x86_64")]
    v.extend([
        libc::SYS_iopl,
        libc::SYS_ioperm,
        libc::SYS_uselib,
        libc::SYS_modify_ldt,
    ]);
    v
}

/// Die `CLONE_NEW*`-Flags.
const CLONE_NEW: [u64; 8] = [
    libc::CLONE_NEWNS as u64,
    libc::CLONE_NEWCGROUP as u64,
    libc::CLONE_NEWUTS as u64,
    libc::CLONE_NEWIPC as u64,
    libc::CLONE_NEWUSER as u64,
    libc::CLONE_NEWPID as u64,
    libc::CLONE_NEWNET as u64,
    0x80, // CLONE_NEWTIME
];

/// x32-Syscalls tragen dieses Bit in der Nummer.
const X32_SYSCALL_BIT: u32 = 0x4000_0000;

fn arch() -> TargetArch {
    if cfg!(target_arch = "aarch64") {
        TargetArch::aarch64
    } else if cfg!(target_arch = "riscv64") {
        TargetArch::riscv64
    } else {
        TargetArch::x86_64
    }
}

fn cond(arg: u8, len: Len, op: Op, value: u64) -> Result<SeccompCondition, String> {
    SeccompCondition::new(arg, len, op, value).map_err(|e| e.to_string())
}

fn rule(conds: Vec<SeccompCondition>) -> Result<SeccompRule, String> {
    SeccompRule::new(conds).map_err(|e| e.to_string())
}

/// Die Filter für eine Landlock-ABI (in dieser Reihenfolge zu laden).
pub fn filters(landlock_abi: u32) -> Result<Vec<BpfProgram>, String> {
    let mut eperm: BTreeMap<i64, Vec<SeccompRule>> =
        denied().into_iter().map(|n| (n, Vec::new())).collect();
    let mut clone = Vec::new();
    for f in CLONE_NEW {
        clone.push(rule(vec![cond(0, Len::Qword, Op::MaskedEq(f), f)?])?);
    }
    eperm.insert(libc::SYS_clone, clone);
    eperm.insert(
        libc::SYS_socket,
        vec![rule(vec![
            cond(0, Len::Dword, Op::Ne, libc::AF_UNIX as u64)?,
            cond(0, Len::Dword, Op::Ne, libc::AF_INET as u64)?,
            cond(0, Len::Dword, Op::Ne, libc::AF_INET6 as u64)?,
        ])?],
    );
    eperm.insert(
        libc::SYS_ioctl,
        vec![
            rule(vec![cond(
                1,
                Len::Dword,
                Op::Eq,
                u64::from(libc::TIOCSTI as u32),
            )?])?,
            rule(vec![cond(
                1,
                Len::Dword,
                Op::Eq,
                u64::from(libc::TIOCLINUX as u32),
            )?])?,
        ],
    );
    let mut enosys: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
    enosys.insert(libc::SYS_clone3, Vec::new());
    if landlock_abi < 3 {
        let trunc_ro = (libc::O_TRUNC | libc::O_ACCMODE) as u64;
        eperm.insert(libc::SYS_truncate, Vec::new());
        eperm.insert(
            libc::SYS_openat,
            vec![rule(vec![cond(
                2,
                Len::Dword,
                Op::MaskedEq(trunc_ro),
                libc::O_TRUNC as u64,
            )?])?],
        );
        #[cfg(target_arch = "x86_64")]
        eperm.insert(
            libc::SYS_open,
            vec![rule(vec![cond(
                1,
                Len::Dword,
                Op::MaskedEq(trunc_ro),
                libc::O_TRUNC as u64,
            )?])?],
        );
        enosys.insert(libc::SYS_openat2, Vec::new());
    }
    let mut out = Vec::new();
    for (rules, errno) in [(eperm, libc::EPERM), (enosys, libc::ENOSYS)] {
        let f = SeccompFilter::new(
            rules,
            SeccompAction::Allow,
            SeccompAction::Errno(errno as u32),
            arch(),
        )
        .map_err(|e| e.to_string())?;
        let prog: BpfProgram = f
            .try_into()
            .map_err(|e: seccompiler::BackendError| e.to_string())?;
        out.push(prog);
    }
    if cfg!(target_arch = "x86_64") {
        // ld [nr]; jge X32 → ret EPERM; sonst weiter mit dem ersten Filter.
        let prefix = vec![
            sock_filter {
                code: 0x20,
                jt: 0,
                jf: 0,
                k: 0,
            },
            sock_filter {
                code: 0x35,
                jt: 0,
                jf: 1,
                k: X32_SYSCALL_BIT,
            },
            sock_filter {
                code: 0x06,
                jt: 0,
                jf: 0,
                k: libc::SECCOMP_RET_ERRNO | libc::EPERM as u32,
            },
        ];
        let first = out.remove(0);
        out.insert(0, prefix.into_iter().chain(first).collect());
    }
    Ok(out)
}

/// Lädt die Filter in den aufrufenden Thread (setzt `PR_SET_NO_NEW_PRIVS`).
pub fn apply(landlock_abi: u32) -> Result<(), String> {
    for prog in filters(landlock_abi)? {
        seccompiler::apply_filter(&prog).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Ist seccomp-BPF im Kernel verfügbar?
pub fn available() -> bool {
    // SAFETY: prctl ohne Zeiger.
    let r = unsafe { libc::prctl(libc::PR_GET_SECCOMP, 0, 0, 0, 0) };
    r >= 0
        && std::fs::read_to_string("/proc/sys/kernel/seccomp/actions_avail")
            .is_ok_and(|a| a.contains("errno"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sbx_008_filters_compile_for_every_abi() {
        for abi in [2, 3, 4, 6] {
            let f = filters(abi).unwrap();
            assert_eq!(f.len(), 2);
            assert!(f.iter().all(|p| !p.is_empty() && p.len() < 4096));
        }
        // ABI 2 sperrt zusätzlich truncate.
        assert!(filters(2).unwrap()[0].len() > filters(3).unwrap()[0].len());
    }
}
