/* Ferron PGO profiler initialization fix for s390x.
 *
 * The pre-compiled LLVM profiling runtime in Rust's s390x profiler_builtins
 * registers its constructor via the .ctors section. Under QEMU s390x user-mode,
 * .ctors entries are not processed (only .init_array is), so the profiler's
 * atexit handler never runs and no .profraw files are generated.
 *
 * This file provides an equivalent constructor via .init_array (modern
 * convention) so that the profiler initializes correctly under QEMU s390x.
 * On native s390x both constructors run; the second call is a no-op because
 * __llvm_profile_runtime is already set.
 */
extern void __llvm_profile_initialize(void);

__attribute__((constructor(1)))
static void ensure_profiler_init(void) {
    __llvm_profile_initialize();
}
