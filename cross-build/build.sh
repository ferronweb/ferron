#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

SUPPORTED_TARGETS=(
	# GNU targets
	"x86_64-unknown-linux-gnu"
	"i686-unknown-linux-gnu"
	"aarch64-unknown-linux-gnu"
	"armv7-unknown-linux-gnueabihf"
	"riscv64gc-unknown-linux-gnu"
	"s390x-unknown-linux-gnu"
	"powerpc64le-unknown-linux-gnu"
	# musl targets
	"x86_64-unknown-linux-musl"
	"i686-unknown-linux-musl"
	"aarch64-unknown-linux-musl"
	"armv7-unknown-linux-musleabihf"
	"riscv64gc-unknown-linux-musl"
)

usage() {
	cat <<EOF
Usage: $(basename "$0") [OPTIONS] <TARGET>

Build Ferron binaries for a specific Linux target with sysroot support.

Targets:
$(printf '  %s\n' "${SUPPORTED_TARGETS[@]}")

Options:
  -p, --pgo               Enable Profile-Guided Optimization
  -s, --sysroot-dir DIR   Custom sysroot directory (default: auto-detect)
  -o, --output-dir DIR    Output directory (default: dist/)
  -b, --bench-duration SEC Benchmark duration for PGO (default: 30)
  -h, --help              Show this help message

Examples:
  $(basename "$0") x86_64-unknown-linux-gnu
  $(basename "$0") aarch64-unknown-linux-gnu --pgo
  $(basename "$0") -s /opt/sysroots x86_64-unknown-linux-musl

Environment variables:
  RUSTFLAGS               Extra Rust compiler flags (will be extended for PGO)
  SYSROOT_DIR             Override sysroot directory
EOF
}

log_info() {
	echo "[INFO] $*"
}

log_error() {
	echo "[ERROR] $*" >&2
}

log_step() {
	echo ""
	echo "========================================="
	echo "  $*"
	echo "========================================="
	echo ""
}

host_arch() {
	case "$(uname -m)" in
		x86_64) echo "x86_64" ;;
		aarch64) echo "aarch64" ;;
		armv7*) echo "armv7" ;;
		riscv64) echo "riscv64" ;;
		s390x) echo "s390x" ;;
		ppc64le) echo "ppc64le" ;;
		i?86) echo "i686" ;;
		*) echo "$(uname -m)" ;;
	esac
}

# Returns the rustc host target triple (e.g. x86_64-unknown-linux-gnu)
rustc_host_target() {
	rustc -vV 2>/dev/null | grep '^host:' | cut -d' ' -f2
}

# Ad hoc conversion from rust target to llvm target
rust_to_llvm_target() {
    local llvm_target
    llvm_target="$1"
    # clang doesn't recognize riscv64gc, so convert to riscv64
    llvm_target=$(echo "${llvm_target}" | sed 's/riscv64gc/riscv64/g')
    echo "${llvm_target}"
}

detect_libc() {
	local target="$1"
	case "${target}" in
		*-gnu*) echo "gnu" ;;
		*-musl*) echo "musl" ;;
		*) log_error "Cannot detect libc for target: ${target}"; return 1 ;;
	esac
}

target_to_arch() {
	local target="$1"
	echo "${target}" | cut -d'-' -f1
}

target_to_deb_arch() {
	local target="$1"
	case "$target" in
		x86_64-unknown-linux-gnu) echo "amd64" ;;
		i686-unknown-linux-gnu) echo "i386" ;;
		aarch64-unknown-linux-gnu) echo "arm64" ;;
		armv7-unknown-linux-gnueabihf) echo "armhf" ;;
		riscv64gc-unknown-linux-gnu) echo "riscv64" ;;
		s390x-unknown-linux-gnu) echo "s390x" ;;
		powerpc64le-unknown-linux-gnu) echo "ppc64el" ;;
		*) echo "" ;;
	esac
}

detect_sysroot() {
	local target="$1"
	local libc
	libc=$(detect_libc "${target}")
	local arch
	arch=$(target_to_arch "${target}")
	local script_dir="${SCRIPT_DIR}/sysroots/prepared"

	# Check for sysroot in standard locations
	local candidates=(
		"${script_dir}/${libc}-${arch}"
		"${script_dir}/${libc}-${target}"
		"${script_dir}/${target}"
	)

	for candidate in "${candidates[@]}"; do
		if [[ -d "${candidate}" ]]; then
			echo "${candidate}"
			return 0
		fi
	done

	return 1
}

target_to_gnu_cross_prefix() {
	local target="$1"
	case "${target}" in
		x86_64-unknown-linux-gnu) echo "" ;;
		i686-unknown-linux-gnu) echo "i686-linux-gnu-" ;;
		aarch64-unknown-linux-gnu) echo "aarch64-linux-gnu-" ;;
		armv7-unknown-linux-gnueabihf) echo "arm-linux-gnueabihf-" ;;
		riscv64gc-unknown-linux-gnu) echo "riscv64-linux-gnu-" ;;
		s390x-unknown-linux-gnu) echo "s390x-linux-gnu-" ;;
		powerpc64le-unknown-linux-gnu) echo "powerpc64le-linux-gnu-" ;;
		*) echo "" ;;
	esac
}

# GNU tuple used for library directories (usr/lib/<gnu_arch>/, /lib/<gnu_arch>/)
target_to_gnu_arch() {
	local target="$1"
	case "${target}" in
		x86_64-unknown-linux-gnu) echo "x86_64-linux-gnu" ;;
		i686-unknown-linux-gnu) echo "i386-linux-gnu" ;;
		aarch64-unknown-linux-gnu) echo "aarch64-linux-gnu" ;;
		armv7-unknown-linux-gnueabihf) echo "arm-linux-gnueabihf" ;;
		riscv64gc-unknown-linux-gnu) echo "riscv64-linux-gnu" ;;
		s390x-unknown-linux-gnu) echo "s390x-linux-gnu" ;;
		powerpc64le-unknown-linux-gnu) echo "powerpc64le-linux-gnu" ;;
		*) echo "" ;;
	esac
}

target_to_qemu_binary() {
	local target="$1"
	case "${target}" in
		x86_64-unknown-linux-gnu | x86_64-unknown-linux-musl) echo "qemu-x86_64-static" ;;
		i686-unknown-linux-gnu | i686-unknown-linux-musl) echo "qemu-i386-static" ;;
		aarch64-unknown-linux-gnu | aarch64-unknown-linux-musl) echo "qemu-aarch64-static" ;;
		armv7-unknown-linux-gnueabihf | armv7-unknown-linux-musleabihf) echo "qemu-arm-static" ;;
		riscv64gc-unknown-linux-gnu | riscv64gc-unknown-linux-musl) echo "qemu-riscv64-static" ;;
		s390x-unknown-linux-gnu) echo "qemu-s390x-static" ;;
		powerpc64le-unknown-linux-gnu) echo "qemu-ppc64le-static" ;;
		*) echo "" ;;
	esac
}

ensure_qemu_user() {
	local target="$1"
	local qemu_binary
	qemu_binary=$(target_to_qemu_binary "${target}")

	if [[ -z "${qemu_binary}" ]]; then
		return 0
	fi

	if command -v "${qemu_binary}" &>/dev/null; then
		return 0
	fi

	log_info "qemu-user-static not found, attempting to install..."

	# Detect distro and install
	if command -v pacman &>/dev/null; then
		# Arch Linux
		if command -v sudo &>/dev/null; then
			sudo pacman -S --noconfirm qemu-user-static qemu-user-static-binfmt
		else
			pacman -S --noconfirm qemu-user-static qemu-user-static-binfmt
		fi
	elif command -v apt-get &>/dev/null; then
		# Debian/Ubuntu
		sudo apt-get update -qq
		sudo apt-get install -y -qq qemu-user-static
	elif command -v dnf &>/dev/null; then
		# Fedora
		sudo dnf install -y qemu-user-static
	elif command -v apk &>/dev/null; then
		# Alpine
		sudo apk add --no-cache qemu-user-static
	else
		log_error "Cannot auto-install qemu-user-static for your distro"
		log_error "Please install manually:"
		log_error "  Arch:       sudo pacman -S qemu-user-static qemu-user-static-binfmt"
		log_error "  Debian/Ubuntu: sudo apt install qemu-user-static"
		log_error "  Fedora:     sudo dnf install qemu-user-static"
		log_error "  Alpine:     sudo apk add qemu-user-static"
		return 1
	fi

	# Verify installation
	if ! command -v "${qemu_binary}" &>/dev/null; then
		log_error "qemu-user-static installed but ${qemu_binary} not found in PATH"
		return 1
	fi

	log_info "qemu-user-static installed successfully"
}

setup_env_gnu() {
	local target="$1"
	local sysroot="$2"
	local cross_prefix
	cross_prefix=$(target_to_gnu_cross_prefix "${target}")
	local gnu_arch
	gnu_arch=$(target_to_gnu_arch "${target}")
	local host
	host=$(host_arch)
	local llvm_target
	llvm_target=$(rust_to_llvm_target "${target}")

	log_info "Setting up GNU build environment"
	log_info "  Sysroot: ${sysroot}"
	log_info "  Cross prefix: ${cross_prefix:-native}"

	# Create clang wrappers
		log_info "Creating clang wrappers for ${target}"

		local wrapper_dir="${sysroot}/bin"
		mkdir -p "${wrapper_dir}"
		local cc_wrapper="${wrapper_dir}/clang-cc"
		local cxx_wrapper="${wrapper_dir}/clang-cxx"
		local ar_wrapper="${wrapper_dir}/clang-ar"
		local ranlib_wrapper="${wrapper_dir}/clang-ranlib"
		local gnu_arch
		gnu_arch=$(echo "${target}" | cut -d'-' -f1)
		local deb_arch
		deb_arch=$(target_to_deb_arch "${target}" 2>/dev/null || echo "${gnu_arch}")

		# Determine GCC install dir in the sysroot
		local gcc_install_dir=""
		for candidate in \
			"${sysroot}/usr/lib/gcc/${gnu_arch}" \
			"${sysroot}/usr/lib/${cross_prefix}${gnu_arch}/gcc/${gnu_arch}" \
			"${sysroot}/usr/lib/${gnu_arch}/gcc/${gnu_arch}"; do
			if [[ -d "${candidate}" ]]; then
				local newest
				newest=$(ls -d "${candidate}"/*/ 2>/dev/null | sort -V | tail -1)
				if [[ -n "${newest}" ]]; then
					gcc_install_dir="${newest%/}"
					break
				fi
			fi
		done

		# Fallback: search for any GCC dir
		if [[ -z "${gcc_install_dir}" ]]; then
			gcc_install_dir=$(find "${sysroot}" -name "crtbeginS.o" -path "*/gcc/*" 2>/dev/null | head -1 | xargs dirname 2>/dev/null || echo "")
		fi

		if [[ -z "${gcc_install_dir}" ]]; then
			log_error "Could not find GCC installation directory in sysroot"
			log_error "Ensure prepare-gnu.sh has been run for ${target}"
			return 1
		fi

		log_info "  GCC install dir: ${gcc_install_dir}"

		local dynamic_linker_path="/lib/${gnu_arch}-linux-gnu/ld-linux-${gnu_arch}.so"
		# find dynamic linker extension (.2, .1)
		local dynamic_linker_filename
		dynamic_linker_filename=$((ls -1 ${sysroot}/lib/ld*.so* 2>/dev/null || true) | tail -n 1 | sed 's|.*/||')
		if [[ -z "${dynamic_linker_filename}" ]]; then
			dynamic_linker_filename=$(ls -1 ${sysroot}/lib/${gnu_arch}-linux-gnu/ld*.so* 2>/dev/null | tail -n 1 | sed 's|.*/||')
			if [[ -n "${dynamic_linker_filename}" ]]; then
				dynamic_linker_path="/lib/${gnu_arch}-linux-gnu/${dynamic_linker_filename}"
			fi
		else
			if [[ -n "${dynamic_linker_filename}" ]]; then
				dynamic_linker_path="/lib/${dynamic_linker_filename}"
			fi
		fi

		# CC wrapper: compile + link, strip profiling flags (only Rust needs PGO instrumentation)
		cat > "${cc_wrapper}" <<WRAPPER
#!/bin/bash
# Strip profiling flags — C dependencies don't need PGO instrumentation
filtered_args=""
for arg in "\$@"; do
    case "\$arg" in
        -fprofile-generate=*|-fprofile-instr-generate=*|-fprofile-instr-use=*|-fprofile-use=*)
            continue ;;
        *)
            filtered_args="\$filtered_args \$arg" ;;
    esac
done
exec clang --sysroot="${sysroot}" --target=${llvm_target} --gcc-install-dir="${gcc_install_dir}" -fuse-ld=lld -Wl,-dynamic-linker=${dynamic_linker_path} \$filtered_args
WRAPPER
		# CXX wrapper: compile + link, strip profiling flags
		cat > "${cxx_wrapper}" <<WRAPPER
#!/bin/bash
filtered_args=""
for arg in "\$@"; do
    case "\$arg" in
        -fprofile-generate=*|-fprofile-instr-generate=*|-fprofile-instr-use=*|-fprofile-use=*)
            continue ;;
        *)
            filtered_args="\$filtered_args \$arg" ;;
    esac
done
exec clang++ --sysroot="${sysroot}" --target=${llvm_target} --gcc-install-dir="${gcc_install_dir}" -fuse-ld=lld -Wl,-dynamic-linker=${dynamic_linker_path} \$filtered_args
WRAPPER
		# AR/RANLIB wrappers
		cat > "${ar_wrapper}" <<WRAPPER
#!/bin/bash
exec llvm-ar "\$@"
WRAPPER
		cat > "${ranlib_wrapper}" <<WRAPPER
#!/bin/bash
exec llvm-ranlib "\$@"
WRAPPER
		chmod +x "${cc_wrapper}" "${cxx_wrapper}" "${ar_wrapper}" "${ranlib_wrapper}"

		# Linker wrapper: use --gcc-install-dir to prevent clang from finding host cross-GCC specs
		# --gcc-install-dir tells clang to use CRT files from our sysroot, not from the host
		local linker_wrapper="${wrapper_dir}/clang-linker"
		# For native builds (target == rustc host), point the dynamic linker at the
		# host loader so the resulting binaries (including build scripts) can RUN on the
		# build machine. The binary is still compiled/linked against the Debian glibc
		# 2.31 sysroot, so it remains portable to any glibc >= 2.31 system.
		local rustc_host
		rustc_host=$(rustc_host_target)
		if [[ "${target}" == "${rustc_host}" && -e "/lib64/ld-linux-x86-64.so.2" ]]; then
			dynamic_linker_path="/lib64/ld-linux-x86-64.so.2"
		fi
		cat > "${linker_wrapper}" <<WRAPPER
#!/bin/bash
exec clang --sysroot="${sysroot}" --target=${llvm_target} --gcc-install-dir="${gcc_install_dir}" -fuse-ld=lld -Wl,-dynamic-linker=${dynamic_linker_path} "\$@"
WRAPPER
		chmod +x "${linker_wrapper}"

		# Write .cargo/config.toml for this target (no empty rustflags)
		mkdir -p "${PROJECT_ROOT}/.cargo"
		local cargo_config="${PROJECT_ROOT}/.cargo/config.toml"
		local linker_line="linker = \"${linker_wrapper}\""

		if [[ ! -f "${cargo_config}" ]]; then
			echo "[target.${target}]" > "${cargo_config}"
			echo "${linker_line}" >> "${cargo_config}"
		else
			if ! grep -q "\[target\.${target}\]" "${cargo_config}"; then
				echo "" >> "${cargo_config}"
				echo "[target.${target}]" >> "${cargo_config}"
				echo "${linker_line}" >> "${cargo_config}"
			fi
		fi

		cc_bin="${cc_wrapper}"
		cxx_bin="${cxx_wrapper}"
		ar_bin="${ar_wrapper}"
		ranlib_bin="${ranlib_wrapper}"

	export CC_${target//-/_}="${cc_bin}"
	export CXX_${target//-/_}="${cxx_bin}"
	export AR_${target//-/_}="${ar_bin}"
	export RANLIB_${target//-/_}="${ranlib_bin}"

	# For bindgen
	export BINDGEN_EXTRA_CLANG_ARGS="--sysroot=${sysroot} --target=${llvm_target} -I${sysroot}/include -I${sysroot}/usr/include"

	local cc_var="CC_${target//-/_}"
	local cxx_var="CXX_${target//-/_}"
	log_info "  CC: ${!cc_var}"
	log_info "  CXX: ${!cxx_var}"
}

setup_env_musl() {
	local target="$1"
	local sysroot="$2"
	local llvm_target
	llvm_target=$(rust_to_llvm_target "${target}")

	log_info "Setting up musl build environment"
	log_info "  Sysroot: ${sysroot}"

	# Use clang for musl targets (matches the project's Dockerfile approach)
	if ! command -v clang &>/dev/null; then
		log_error "clang not found. Required for musl targets."
		log_error "Install clang:"
		log_error "  Arch:       sudo pacman -S clang"
		log_error "  Debian/Ubuntu: sudo apt install clang"
		log_error "  Fedora:     sudo dnf install clang"
		return 1
	fi

	# Copy self-contained libunwind from Rust toolchain (matches Dockerfile approach)
	local rust_libdir
	rust_libdir=$(rustc --print target-libdir --target "${target}")
	local unwind_src="${rust_libdir}/self-contained/libunwind.a"
	local unwind_dest="${sysroot}/lib/libunwind.a"

	if [[ -f "${unwind_src}" ]]; then
		if [[ ! -f "${unwind_dest}" ]]; then
			log_info "Copying self-contained libunwind from Rust toolchain"
			cp "${unwind_src}" "${unwind_dest}"
		fi
	fi

	# Rust's musl target links with -lgcc_s (shared), but a static-pie musl
	# binary has no shared libgcc_s. Provide a GNU linker script that redirects
	# -lgcc_s to the static libgcc.a; the _Unwind_* (exception/unwind) symbols
	# are supplied by libunwind.a, so include it as well. This lets both the
	# final Rust link and CMake compiler probes resolve -lgcc_s under lld.
	if [[ -f "${sysroot}/lib/libgcc.a" ]]; then
		local gcc_s_script="${sysroot}/lib/libgcc_s.so"
		if [[ -f "${sysroot}/lib/libunwind.a" ]]; then
			echo 'INPUT(libgcc.a libunwind.a)' > "${gcc_s_script}"
		else
			echo 'INPUT(libgcc.a)' > "${gcc_s_script}"
		fi
		log_info "  Created libgcc_s.so linker script -> libgcc.a[+libunwind.a]"
	fi

	# Use clang with explicit sysroot and target for musl
	export AR_${target//-/_}="llvm-ar"
	export RANLIB_${target//-/_}="llvm-ranlib"

	# Create CC/CXX wrapper scripts for build scripts (aws-lc-sys, etc.)
	# that parse CC as a single binary path
	local wrapper_dir="${sysroot}/bin"
	mkdir -p "${wrapper_dir}"
	local cc_wrapper="${wrapper_dir}/clang-cc"
	local cxx_wrapper="${wrapper_dir}/clang-cxx"
	# C build scripts (aws-lc-sys/CMake) don't need PGO instrumentation, and
	# passing -fprofile-generate/-fprofile-use through makes CMake's compiler
	# probes link the HOST libgcc_s and fail. Strip profiling flags (Rust
	# handles its own instrumentation via RUSTFLAGS) and only add -fuse-ld=lld
	# when actually linking (no -c), so pure compiles don't trip
	# -Werror,-Wunused-command-line-argument.
	cat > "${cc_wrapper}" <<WRAPPER
#!/bin/bash
# Strip profiling flags — C dependencies don't need PGO instrumentation
filtered_args=""
link=1
for a in "\$@"; do
  case "\$a" in
    -fprofile-generate=*|-fprofile-instr-generate=*|-fprofile-instr-use=*|-fprofile-use*) continue ;;
    -c|-E|-S|-M*) link=0 ;;
  esac
  filtered_args="\$filtered_args \$a"
done
if [[ \$link -eq 1 ]]; then
  exec clang --sysroot=${sysroot} --gcc-install-dir="${sysroot}/lib" --target=${llvm_target} -fuse-ld=lld \$filtered_args
else
  exec clang --sysroot=${sysroot} --gcc-install-dir="${sysroot}/lib" --target=${llvm_target} \$filtered_args
fi
WRAPPER
	cat > "${cxx_wrapper}" <<WRAPPER
#!/bin/bash
filtered_args=""
link=1
for a in "\$@"; do
  case "\$a" in
    -fprofile-generate=*|-fprofile-instr-generate=*|-fprofile-instr-use=*|-fprofile-use*) continue ;;
    -c|-E|-S|-M*) link=0 ;;
  esac
  filtered_args="\$filtered_args \$a"
done
if [[ \$link -eq 1 ]]; then
  exec clang++ --sysroot=${sysroot} --gcc-install-dir="${sysroot}/lib" --target=${llvm_target} -fuse-ld=lld \$filtered_args
else
  exec clang++ --sysroot=${sysroot} --gcc-install-dir="${sysroot}/lib" --target=${llvm_target} \$filtered_args
fi
WRAPPER
	chmod +x "${cc_wrapper}" "${cxx_wrapper}"

	export CC="${cc_wrapper}"
	export CXX="${cxx_wrapper}"

	# Tell Cargo to use clang as the linker for this target (clang + lld, no GCC)
	mkdir -p "${PROJECT_ROOT}/.cargo"
	local cargo_config="${PROJECT_ROOT}/.cargo/config.toml"
	local linker_wrapper="${wrapper_dir}/musl-gcc-linker"

	# Build sysroot lib paths for linker
	local sysroot_lib_args="-L${sysroot}/lib"

	# Use clang as linker with lld (LLVM) — avoids any GNU binutils ld
	# dependency and keeps the build fully clang/lld-based
	cat > "${linker_wrapper}" <<WRAPPER
#!/bin/bash
exec clang --target=${llvm_target} --sysroot="${sysroot}" --gcc-install-dir="${sysroot}/lib" -fuse-ld=lld "\$@"
WRAPPER
	chmod +x "${linker_wrapper}"

	local linker_line="linker = \"${linker_wrapper}\""
	local rustflags_linker=""

	# Create or update .cargo/config.toml with target-specific linker
	if [[ ! -f "${cargo_config}" ]]; then
		echo "[target.${target}]" > "${cargo_config}"
		echo "${linker_line}" >> "${cargo_config}"
		echo "rustflags = [\"${rustflags_linker}\"]" >> "${cargo_config}"
	else
		# Check if target section already exists
		if ! grep -q "\[target\.${target}\]" "${cargo_config}"; then
			echo "" >> "${cargo_config}"
			echo "[target.${target}]" >> "${cargo_config}"
			echo "${linker_line}" >> "${cargo_config}"
			echo "rustflags = [\"${rustflags_linker}\"]" >> "${cargo_config}"
		fi
	fi

	# CXXFLAGS matching the project's Dockerfile
	export CXXFLAGS="-U TCMALLOC_INTERNAL_METHODS_ONLY -isystem ${sysroot}/include -I${sysroot}/include/c++/v1 -stdlib=libc++ -std=c++17 -nostdinc++ -static --target=${llvm_target}"

	# Tell Rust to use libc++ instead of libstdc++
	export CXXSTDLIB="c++"

	# For bindgen
	export BINDGEN_EXTRA_CLANG_ARGS="--sysroot=${sysroot} --target=${llvm_target} -I${sysroot}/include"

	# Rust flags for musl self-contained linking (matches previous Dockerfile approach)
	export RUSTFLAGS="${RUSTFLAGS:-} -Clink-self-contained=no -Clink-args=-L${sysroot}/lib -Clink-args=-lc++abi -Ctarget-feature=+crt-static"

	# musl-specific env
	export RUST_LIBC_UNSTABLE_MUSL_V1_2_3=1

	log_info "  CC: ${cc_wrapper}"
	log_info "  CXX: ${cxx_wrapper}"
	log_info "  Linker: clang + lld (no GCC)"
	log_info "  CXXSTDLIB: c++"
}

check_rust_target() {
	local target="$1"
	local installed_targets
	installed_targets=$(rustup target list --installed)

	if echo "${installed_targets}" | grep -q "^${target}$"; then
		return 0
	fi

	log_info "Installing Rust target: ${target}"
	rustup target add "${target}"
}

pgo_build() {
	local target="$1"
	local pgo_data_dir="/tmp/ferron-pgo-data-$$"
	local bench_duration="$2"

	log_step "Phase 1: Building with PGO instrumentation"

	mkdir -p "${pgo_data_dir}"

	local instrument_rustflags="-Cprofile-generate=${pgo_data_dir}"
	# Append to (not replace) the existing RUSTFLAGS so target-specific flags
	# set by setup_env_* (e.g. musl's -lc++abi / -L<sysroot>/lib) are preserved.
	RUSTFLAGS="${RUSTFLAGS:-} ${instrument_rustflags}" cargo build --release --target "${target}" \
		--manifest-path "${PROJECT_ROOT}/Cargo.toml"

	log_step "Phase 2: Running PGO training benchmarks"

	local built_binary="${PROJECT_ROOT}/target/${target}/release/ferron"
	if [[ ! -f "${built_binary}" ]]; then
		log_error "Built binary not found: ${built_binary}"
		return 1
	fi

	# Ensure qemu is available if cross-compiling
	local host
	host=$(host_arch)
	local target_arch
	target_arch=$(target_to_arch "${target}")

	# Set QEMU_LD_PREFIX for cross-compiled binaries
	if [[ "${host}" != "${target_arch}" ]]; then
		ensure_qemu_user "${target}"
		local sysroot
		sysroot=$(detect_sysroot "${target}")
		export QEMU_LD_PREFIX="${sysroot}"
		log_info "  QEMU_LD_PREFIX: ${QEMU_LD_PREFIX}"
	fi

	# Run the benchmark script
	local bench_script="${SCRIPT_DIR}/benchmarks/run.sh"
	if [[ -x "${bench_script}" ]]; then
		"${bench_script}" "${built_binary}" "${target}" --duration "${bench_duration}" --pgo-data-dir "${pgo_data_dir}"
	else
		log_error "Benchmark script not found or not executable: ${bench_script}"
		log_error "Run benchmarks manually, then merge profiles:"
		log_error "  llvm-profdata merge -output=${pgo_data_dir}/merged.profdata ${pgo_data_dir}/*.profraw"
		return 1
	fi

	log_step "Phase 3: Merging PGO profiles"

	local profdata_count
	profdata_count=$(find "${pgo_data_dir}" -name "*.profraw" 2>/dev/null | wc -l)
	if [[ "${profdata_count}" -eq 0 ]]; then
		log_error "No .profraw files found in ${pgo_data_dir}"
		log_error "Ensure the benchmark ran successfully"
		return 1
	fi

	if ! command -v llvm-profdata &>/dev/null; then
		log_error "llvm-profdata not found. Required for PGO profile merging."
		log_error "Install llvm:"
		log_error "  Arch:       sudo pacman -S llvm"
		log_error "  Debian/Ubuntu: sudo apt install llvm"
		log_error "  Fedora:     sudo dnf install llvm"
		return 1
	fi

	llvm-profdata merge \
		-output="${pgo_data_dir}/merged.profdata" \
		"${pgo_data_dir}"/*.profraw

	log_info "Merged profile: ${pgo_data_dir}/merged.profdata"

	log_step "Phase 4: Building with PGO-guided optimization"

	local optimize_rustflags="-Cprofile-use=${pgo_data_dir}/merged.profdata"
	# Append to (not replace) the existing RUSTFLAGS — see phase 1 note.
	RUSTFLAGS="${RUSTFLAGS:-} ${optimize_rustflags}" cargo build --release --target "${target}" \
		--manifest-path "${PROJECT_ROOT}/Cargo.toml"

	log_info "PGO build complete"

	# Cleanup PGO data (keep if debug flag is set)
	if [[ "${FERRON_KEEP_PGO_DATA:-0}" != "1" ]]; then
		log_info "Cleaning up PGO data: ${pgo_data_dir}"
		rm -rf "${pgo_data_dir}"
	fi
}

regular_build() {
	local target="$1"

	log_step "Building Ferron"

	cargo build --release --target "${target}" \
		--manifest-path "${PROJECT_ROOT}/Cargo.toml"

	log_info "Build complete"
}

copy_binaries() {
	local target="$1"
	local output_dir="$2"
	local target_dir="${PROJECT_ROOT}/target/${target}/release"
	local binaries=(
		"ferron"
		"ferron-fmt"
		"ferron-passwd"
		"ferron-precompress"
		"ferron-kdl2ferron"
		"ferron-serve"
	)

	mkdir -p "${output_dir}"

	for bin in "${binaries[@]}"; do
		local src="${target_dir}/${bin}"
		if [[ -f "${src}" ]]; then
			cp "${src}" "${output_dir}/"
			log_info "  Copied: ${bin}"
		else
			log_info "  Skipped (not found): ${bin}"
		fi
	done

	# Copy config and webroot if present
	if [[ -f "${PROJECT_ROOT}/configs/ferron.release.conf" ]]; then
		cp "${PROJECT_ROOT}/configs/ferron.release.conf" "${output_dir}/ferron.conf"
	fi
	if [[ -d "${PROJECT_ROOT}/wwwroot" ]]; then
		cp -r "${PROJECT_ROOT}/wwwroot" "${output_dir}/"
	fi
}

main() {
	local target=""
	local pgo=false
	local sysroot_dir=""
	local output_dir="${PROJECT_ROOT}/dist"
	local bench_duration=30

	while [[ $# -gt 0 ]]; do
		case "$1" in
			-p | --pgo)
				pgo=true
				shift
				;;
			-s | --sysroot-dir)
				sysroot_dir="$2"
				shift 2
				;;
			-o | --output-dir)
				output_dir="$2"
				shift 2
				;;
			-b | --bench-duration)
				bench_duration="$2"
				shift 2
				;;
			-h | --help)
				usage
				exit 0
				;;
			-*)
				log_error "Unknown option: $1"
				usage >&2
				exit 1
				;;
			*)
				target="$1"
				shift
				;;
		esac
	done

	if [[ -z "${target}" ]]; then
		log_error "No target specified"
		usage >&2
		exit 1
	fi

	# Validate target
	local valid=false
	for supported in "${SUPPORTED_TARGETS[@]}"; do
		if [[ "${target}" == "${supported}" ]]; then
			valid=true
			break
		fi
	done

	if [[ "${valid}" != "true" ]]; then
		log_error "Unsupported target: ${target}"
		log_error "Supported targets: ${SUPPORTED_TARGETS[*]}"
		exit 1
	fi

	log_step "Building Ferron for ${target}"

	# Detect sysroot
	if [[ -z "${sysroot_dir}" ]]; then
		if [[ -n "${SYSROOT_DIR:-}" ]]; then
			sysroot_dir="${SYSROOT_DIR}"
		elif ! sysroot_dir=$(detect_sysroot "${target}"); then
			log_error "Sysroot not found for ${target}"
			log_error ""
			log_error "Prepare the sysroot first:"
			local libc
			libc=$(detect_libc "${target}")
			log_error "  ./cross-build/sysroots/prepare-${libc}.sh ${target}"
			log_error ""
			log_error "Or specify the sysroot directory:"
			log_error "  $(basename "$0") --sysroot-dir /path/to/sysroot ${target}"
			exit 1
		fi
	fi

	# Verify sysroot exists
	if [[ ! -d "${sysroot_dir}" ]]; then
		log_error "Sysroot directory does not exist: ${sysroot_dir}"
		exit 1
	fi

	# Ensure Rust target is installed
	check_rust_target "${target}"

	# Setup build environment
	local libc
	libc=$(detect_libc "${target}")

	if [[ "${libc}" == "gnu" ]]; then
		setup_env_gnu "${target}" "${sysroot_dir}"
	else
		setup_env_musl "${target}" "${sysroot_dir}"
	fi

	# Build
	if [[ "${pgo}" == "true" ]]; then
		pgo_build "${target}" "${bench_duration}"
	else
		regular_build "${target}"
	fi

	# Copy binaries
	local target_output="${output_dir}/${target}"
	mkdir -p "${target_output}"
	copy_binaries "${target}" "${target_output}"

	log_step "Build Summary"
	log_info "Target: ${target}"
	log_info "Libc: ${libc}"
	log_info "Sysroot: ${sysroot_dir}"
	log_info "PGO: ${pgo}"
	log_info "Output: ${target_output}"
	log_info ""
	log_info "Binaries:"
	ls -lh "${target_output}/" | grep -v "^total" | awk '{print "  " $NF " (" $5 ")"}'
}

main "$@"
