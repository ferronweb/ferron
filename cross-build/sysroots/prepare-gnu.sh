#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEFAULT_OUTPUT_DIR="${SCRIPT_DIR}/../sysroots/prepared"

SUPPORTED_TARGETS=(
	"x86_64-unknown-linux-gnu"
	"i686-unknown-linux-gnu"
	"aarch64-unknown-linux-gnu"
	"armv7-unknown-linux-gnueabihf"
	"riscv64gc-unknown-linux-gnu"
	"s390x-unknown-linux-gnu"
	"powerpc64le-unknown-linux-gnu"
)

DEBIAN_MIRROR="https://deb.debian.org/debian"
DEBIAN_SECURITY="https://security.debian.org/debian-security"
DEBIAN_COMPONENTS="main"

usage() {
	cat <<EOF
Usage: $(basename "$0") [OPTIONS] <TARGET>

Create a Debian-based sysroot with glibc for GNU targets.

Targets:
$(printf '  %s\n' "${SUPPORTED_TARGETS[@]}")

Options:
  -o, --output-dir DIR    Output directory (default: ${DEFAULT_OUTPUT_DIR})
  -s, --suite SUIT        Debian suite (default: autodetect)
  -h, --help              Show this help message

Examples:
  $(basename "$0") x86_64-unknown-linux-gnu
  $(basename "$0") aarch64-unknown-linux-gnu
  $(basename "$0") --all
EOF
}

log_info() {
	echo "[INFO] $*"
}

log_error() {
	echo "[ERROR] $*" >&2
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
		*) log_error "Unknown target: $target"; return 1 ;;
	esac
}

target_to_deb_suite() {
	local target="$1"
	case "$target" in
		x86_64-unknown-linux-gnu) echo "bullseye" ;; # amd64
		i686-unknown-linux-gnu) echo "bullseye" ;; # i386
		aarch64-unknown-linux-gnu) echo "bullseye" ;; # arm64
		armv7-unknown-linux-gnueabihf) echo "bullseye" ;; # armhf
		riscv64gc-unknown-linux-gnu) echo "trixie" ;; # riscv64
		s390x-unknown-linux-gnu) echo "bookworm" ;; # s390x
		powerpc64le-unknown-linux-gnu) echo "bookworm" ;; # ppc64el
		*) log_error "Unknown target: $target"; return 1 ;;
	esac
}

target_to_fallback_gcc() {
	local target="$1"
	case "$target" in
		x86_64-unknown-linux-gnu) echo "10" ;; # bullseye
		i686-unknown-linux-gnu) echo "10" ;; # bullseye
		aarch64-unknown-linux-gnu) echo "10" ;; # bullseye
		armv7-unknown-linux-gnueabihf) echo "10" ;; # bullseye
		riscv64gc-unknown-linux-gnu) echo "12" ;; # trixie
		s390x-unknown-linux-gnu) echo "11" ;; # bookworm
		powerpc64le-unknown-linux-gnu) echo "11" ;; # bookworm
		*) log_error "Unknown target: $target"; return 1 ;;
	esac
}


# GNU tuple used for library directories (usr/lib/<gnu_arch>/, /lib/<gnu_arch>/)
target_to_gnu_arch() {
	local target="$1"
	case "$target" in
		x86_64-unknown-linux-gnu) echo "x86_64-linux-gnu" ;;
		i686-unknown-linux-gnu) echo "i386-linux-gnu" ;;
		aarch64-unknown-linux-gnu) echo "aarch64-linux-gnu" ;;
		armv7-unknown-linux-gnueabihf) echo "arm-linux-gnueabihf" ;;
		riscv64gc-unknown-linux-gnu) echo "riscv64-linux-gnu" ;;
		s390x-unknown-linux-gnu) echo "s390x-linux-gnu" ;;
		powerpc64le-unknown-linux-gnu) echo "powerpc64le-linux-gnu" ;;
		*) log_error "Unknown target: $target"; return 1 ;;
	esac
}

target_to_gnu_prefix() {
	local target="$1"
	case "$target" in
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

target_to_qemu_arch() {
	local target="$1"
	case "$target" in
		x86_64-unknown-linux-gnu) echo "x86_64" ;;
		i686-unknown-linux-gnu) echo "i386" ;;
		aarch64-unknown-linux-gnu) echo "aarch64" ;;
		armv7-unknown-linux-gnueabihf) echo "arm" ;;
		riscv64gc-unknown-linux-gnu) echo "riscv64" ;;
		s390x-unknown-linux-gnu) echo "s390x" ;;
		powerpc64le-unknown-linux-gnu) echo "ppc64" ;;
		*) echo "" ;;
	esac
}

# Download and extract a Debian .deb package
extract_deb() {
	local deb_url="$1"
	local dest_dir="$2"
	local tmp_dir
	tmp_dir=$(mktemp -d)
	local deb_file="${tmp_dir}/$(basename "${deb_url}")"

	log_info "  Downloading: ${deb_url}"
	if ! curl -fsSL -o "${deb_file}" "${deb_url}"; then
		rm -rf "${tmp_dir}"
		return 1
	fi

	# .deb files are ar archives containing data.tar.xz
	mkdir -p "${dest_dir}"
	local data_tar="${tmp_dir}/data.tar.xz"
	if ar -p "${deb_file}" data.tar.xz > "${data_tar}" 2>/dev/null; then
		tar -xJf "${data_tar}" -C "${dest_dir}" 2>/dev/null || \
		tar -xf "${data_tar}" -C "${dest_dir}" 2>/dev/null || true
	fi
	rm -rf "${tmp_dir}"
}

# Get package info from a Debian Packages index
get_deb_url() {
	local package="$1"
	local arch="$2"
	local suite="$3"
	local mirror="$4"

	local index_url="${mirror}/dists/${suite}/main/binary-${arch}/Packages.xz"
	local tmp_dir
	tmp_dir=$(mktemp -d)
	local raw_file="${tmp_dir}/Packages.raw"

	# Download and decompress Packages index
	if ! curl -fsSL -o "${raw_file}.xz" "${index_url}" 2>/dev/null; then
		index_url="${mirror}/dists/${suite}/main/binary-${arch}/Packages.gz"
		if ! curl -fsSL -o "${raw_file}.gz" "${index_url}" 2>/dev/null; then
			rm -rf "${tmp_dir}"
			return 1
		fi
		gunzip -f "${raw_file}.gz" 2>/dev/null
		mv "${raw_file}" "${tmp_dir}/Packages.txt"
	elif [[ -f "${raw_file}.xz" ]]; then
		xz -d "${raw_file}.xz" 2>/dev/null
		mv "${raw_file}" "${tmp_dir}/Packages.txt"
	fi

	local index_file="${tmp_dir}/Packages.txt"

	# Extract version and filename for the package
	local result
	result=$(awk -v pkg="${package}" '
		/^Package:/ { current_pkg=$2 }
		/^Version:/ && current_pkg==pkg { ver=$2 }
		/^Filename:/ && current_pkg==pkg { print ver " " $2; current_pkg="" }
	' "${index_file}" | head -1)

	rm -rf "${tmp_dir}"

	if [[ -z "${result}" ]]; then
		return 1
	fi
	echo "${result}"
}

# Build a cross sysroot by downloading and extracting .deb packages directly
prepare_sysroot() {
	local target="$1"
	local output_dir="$2"
	local sysroot_dir="${output_dir}/gnu-${target}"
	local suite="$3"
	local deb_arch
	local prefix
	local fallback_gcc_version

	deb_arch=$(target_to_deb_arch "${target}")
	prefix=$(target_to_gnu_prefix "${target}")
	local gnu_arch
	gnu_arch=$(target_to_gnu_arch "${target}")
	if [[ -z "${suite}" ]]; then
		suite=$(target_to_deb_suite "${target}")
	fi
	fallback_gcc_version="$(target_to_fallback_gcc "${target}")"

	# The Debian CRT package version is determined by the SUITE
	# (fallback_gcc_version), NOT by any compiler installed on the host. A
	# host cross-compiler (e.g. aarch64-linux-gnu-gcc 16) may report a version
	# that does not exist in the target Debian suite, so we must always use the
	# suite version when selecting libgcc-<ver>-dev / libstdc++-<ver>-dev.
	local gcc_version="${fallback_gcc_version}"

	log_info "Preparing GNU sysroot for ${target}"
	log_info "  Debian arch: ${deb_arch}"
	log_info "  Debian suite: ${suite}"
	log_info "  Cross prefix: ${prefix:-native}"
	log_info "  GCC version: ${gcc_version}"
	log_info "  Output: ${sysroot_dir}"

	if [[ -d "${sysroot_dir}" ]]; then
		log_info "Sysroot already exists, removing: ${sysroot_dir}"
		rm -rf "${sysroot_dir}"
	fi

	mkdir -p "${sysroot_dir}"

	local tmp_dir
	tmp_dir=$(mktemp -d)

	local mirror="${DEBIAN_MIRROR}"

	# Download core glibc packages
	log_info "=== Downloading core glibc packages ==="
	for pkg in "libc6" "libc6-dev" "linux-libc-dev"; do
		log_info "Resolving package: ${pkg}"
		local pkg_info
		pkg_info=$(get_deb_url "${pkg}" "${deb_arch}" "${suite}" "${mirror}") || {
			log_error "Could not find package ${pkg} for ${deb_arch} in ${suite}"
			continue
		}
		local pkg_ver pkg_file
		pkg_ver=$(echo "${pkg_info}" | cut -d' ' -f1)
		pkg_file=$(echo "${pkg_info}" | cut -d' ' -f2-)
		log_info "  ${pkg}: ${pkg_ver}"
		extract_deb "${mirror}/${pkg_file}" "${sysroot_dir}"
	done

	# Download libgcc-s1 (provides libgcc_s.so.1 compatible with glibc 2.31)
	log_info "=== Downloading libgcc-s1 ==="
	local pkg_info
	pkg_info=$(get_deb_url "libgcc-s1" "${deb_arch}" "${suite}" "${mirror}") || {
		log_error "Could not find libgcc-s1 for ${deb_arch}"
	}
	if [[ -n "${pkg_info:-}" ]]; then
		local pkg_ver pkg_file
		pkg_ver=$(echo "${pkg_info}" | cut -d' ' -f1)
		pkg_file=$(echo "${pkg_info}" | cut -d' ' -f2-)
		log_info "  libgcc-s1: ${pkg_ver}"
		extract_deb "${mirror}/${pkg_file}" "${sysroot_dir}"
	fi

	# Download libgcc-10-dev (provides CRT files: crtbeginS.o, crtendS.o, libgcc.a)
	log_info "=== Downloading libgcc-${gcc_version}-dev ==="
	pkg_info=$(get_deb_url "libgcc-${gcc_version}-dev" "${deb_arch}" "${suite}" "${mirror}") || {
		log_info "  libgcc-${gcc_version}-dev not found, skipping"
	}
	if [[ -n "${pkg_info:-}" ]]; then
		local pkg_ver pkg_file
		pkg_ver=$(echo "${pkg_info}" | cut -d' ' -f1)
		pkg_file=$(echo "${pkg_info}" | cut -d' ' -f2-)
		log_info "  libgcc-${gcc_version}-dev: ${pkg_ver}"
		extract_deb "${mirror}/${pkg_file}" "${sysroot_dir}"
	fi

	# Download libstdc++6 (shared) and libstdc++-10-dev (static)
	log_info "=== Downloading libstdc++6 ==="
	pkg_info=$(get_deb_url "libstdc++6" "${deb_arch}" "${suite}" "${mirror}") || {
		log_error "Could not find libstdc++6 for ${deb_arch}"
	}
	if [[ -n "${pkg_info:-}" ]]; then
		local pkg_ver pkg_file
		pkg_ver=$(echo "${pkg_info}" | cut -d' ' -f1)
		pkg_file=$(echo "${pkg_info}" | cut -d' ' -f2-)
		log_info "  libstdc++6: ${pkg_ver}"
		extract_deb "${mirror}/${pkg_file}" "${sysroot_dir}"
	fi

	log_info "=== Downloading libstdc++-${gcc_version}-dev ==="
	pkg_info=$(get_deb_url "libstdc++-${gcc_version}-dev" "${deb_arch}" "${suite}" "${mirror}") || {
		log_info "  libstdc++-${gcc_version}-dev not found, skipping"
	}
	if [[ -n "${pkg_info:-}" ]]; then
		local pkg_ver pkg_file
		pkg_ver=$(echo "${pkg_info}" | cut -d' ' -f1)
		pkg_file=$(echo "${pkg_info}" | cut -d' ' -f2-)
		log_info "  libstdc++-${gcc_version}-dev: ${pkg_ver}"
		extract_deb "${mirror}/${pkg_file}" "${sysroot_dir}"
	fi

	rm -rf "${tmp_dir}"

	# Ensure directory structure
	mkdir -p "${sysroot_dir}/usr/lib"
	mkdir -p "${sysroot_dir}/usr/include"
	mkdir -p "${sysroot_dir}/lib"

	local arch_lib_dir="${sysroot_dir}/usr/lib/${gnu_arch}"

	# Create /lib symlinks to /usr/lib/<arch>/ so the linker can find libraries
	if [[ -d "${arch_lib_dir}" ]]; then
		for lib in "${arch_lib_dir}"/*.so* "${arch_lib_dir}"/*.a; do
			if [[ -f "${lib}" || -L "${lib}" ]]; then
				local lib_name
				lib_name=$(basename "${lib}")
				ln -sf "../usr/lib/${gnu_arch}/${lib_name}" "${sysroot_dir}/lib/${lib_name}" 2>/dev/null || true
			fi
		done
	fi

	# Create runtime symlinks from /lib/ for shared objects found in the multiarch
	# lib/<gnu_arch>/ directory. The runtime linker searches /lib/ (and other
	# compiled-in paths) but Debian stores runtime .so.N files under the
	# multiarch path (e.g. /lib/i386-linux-gnu/). Without these symlinks QEMU
	# user-mode falls back to the host's 32-bit libraries (if installed), which
	# may be a different glibc version and cause symbol-version mismatches.
	local gnu_lib_dir="${sysroot_dir}/lib/${gnu_arch}"
	if [[ -d "${gnu_lib_dir}" ]]; then
		for lib in "${gnu_lib_dir}"/*.so.*; do
			if [[ -f "${lib}" || -L "${lib}" ]]; then
				local lib_name
				lib_name=$(basename "${lib}")
				local target_link="${sysroot_dir}/lib/${lib_name}"
				if [[ ! -e "${target_link}" && ! -L "${target_link}" ]]; then
					ln -sf "${gnu_arch}/${lib_name}" "${target_link}" 2>/dev/null || true
				fi
			fi
		done
	fi

	# Fix libc.so linker scripts to use correct dynamic linker path
	log_info "Fixing linker scripts..."
	local libc_linker_script
	libc_linker_script=$(find "${sysroot_dir}" -name "libc.so" -type f 2>/dev/null | head -1)
	if [[ -n "${libc_linker_script}" ]]; then
		log_info "  Found libc.so linker script: ${libc_linker_script}"
		local fixed_script
		fixed_script=$(mktemp)
		local arch_dir
		arch_dir=$(echo "${target}" | cut -d'-' -f1)
		local gnu_arch="${arch_dir}-linux-gnu"
		sed "s|/lib/ld-linux-${arch_dir}\.so|/lib/${gnu_arch}/ld-linux-${arch_dir}.so|g" \
			"${libc_linker_script}" > "${fixed_script}"
		cat "${fixed_script}" > "${libc_linker_script}"
		rm -f "${fixed_script}"
		log_info "  Fixed: $(cat "${libc_linker_script}")"
	fi

	# Fix broken symlinks: .so files in usr/lib/<arch>/ that point to absolute /lib/
	# paths (or to a wrong-depth relative path). These break under --sysroot.
	# Point each linker-name .so at the ACTUAL shared object (.so.N) wherever the
	# Debian package placed it (lib/<arch>/ on bullseye, lib/ on trixie+, etc.).
	log_info "Fixing broken symlinks..."
	if [[ -d "${arch_lib_dir}" ]]; then
		for so_link in "${arch_lib_dir}"/*.so; do
			if [[ -L "${so_link}" ]]; then
				local link_target
				link_target=$(readlink "${so_link}")
				# Broken if absolute (/lib/...), wrong-depth relative (../lib/...), or dangling
				if [[ "${link_target}" == /lib/* || "${link_target}" == ../lib/* || ! -e "${so_link}" ]]; then
					local base_name
					base_name=$(basename "${link_target}")
					# Locate the real shared object under <sysroot>/lib
					local real_so
					real_so=$(find "${sysroot_dir}/lib" -name "${base_name}" 2>/dev/null | head -1)
					if [[ -n "${real_so}" ]]; then
						local rel_path
						rel_path=$(realpath -s --relative-to="${arch_lib_dir}" "${real_so}")
						ln -sf "${rel_path}" "${so_link}" 2>/dev/null || true
					fi
				fi
			fi
		done
	fi

	# Fix ld-script files (libc.so, libm.so, libm.a, etc.) that contain absolute
	# paths in GROUP() directives. Rewrite each absolute /lib/.../FILE or
	# /usr/lib/.../FILE to a relative path pointing at the ACTUAL FILE (which may
	# live in lib/ or lib/<arch>/ depending on the Debian release).
	if [[ -d "${arch_lib_dir}" ]]; then
		local script_file
		while IFS= read -r script_file; do
			[[ -n "${script_file}" ]] || continue
			# Only touch GNU ld scripts (contain GROUP or OUTPUT_FORMAT)
			if grep -qE 'GROUP|OUTPUT_FORMAT' "${script_file}" 2>/dev/null; then
				# Collect absolute paths referenced in the script
				local abs_paths
				abs_paths=$(grep -oE '/(lib|usr/lib)/[^ )]+' "${script_file}" 2>/dev/null | sort -u)
				local abs_path
				for abs_path in ${abs_paths}; do
					local base_name
					base_name=$(basename "${abs_path}")
					local real_file
					real_file=$(find "${sysroot_dir}/lib" -name "${base_name}" 2>/dev/null | head -1)
					if [[ -n "${real_file}" ]]; then
						local rel_path
						rel_path=$(realpath -s --relative-to="${arch_lib_dir}" "${real_file}")
						# Replace the absolute path (token) with the relative path
						# Match the path as a standalone token (followed by space, ) or end)
						sed -i -E "s@${abs_path}([ )])@${rel_path}\1@g; s@${abs_path}\$@${rel_path}@g" "${script_file}" 2>/dev/null
					fi
				done
			fi
		done < <(find "${arch_lib_dir}" -maxdepth 1 \( -name '*.so' -o -name '*.a' \) 2>/dev/null)
	fi

	# Also create .so symlinks in usr/lib/<arch>/ for libraries present as
	# .so.N anywhere under <sysroot>/lib (some Debian arches, e.g. i386, keep
	# the .so.N files in lib/<arch>/ rather than directly in lib/).
	local lib_dir="${sysroot_dir}/lib"
	if [[ -d "${lib_dir}" ]]; then
		while IFS= read -r so_file; do
			if [[ -f "${so_file}" || -L "${so_file}" ]]; then
				local base_name
				base_name=$(basename "${so_file}")
				# Get the short name (e.g. libpthread.so.0 from libpthread.so.0.0)
				local short_name="${base_name%%.*}"
				short_name="${short_name}.so"
				if [[ ! -e "${arch_lib_dir}/${short_name}" ]]; then
					# Point to the actual .so.N wherever it lives under <sysroot>/lib
					local rel_path
					rel_path=$(realpath -s --relative-to="${arch_lib_dir}" "${so_file}")
					ln -sf "${rel_path}" "${arch_lib_dir}/${short_name}" 2>/dev/null || true
				fi
			fi
		done < <(find "${lib_dir}" -name '*.so.*' 2>/dev/null)
	fi

	# Fix broken symlinks for dynamic linker in /lib and /lib64
	for lib_path in "${sysroot_dir}/lib" "${sysroot_dir}/lib64"; do
		if [[ -d "${lib_path}" ]]; then
			for so_file in "${lib_path}"/*.so.*; do
				if [[ -L "${so_file}" ]]; then
					local so_link="${so_file}"
					local link_target
					link_target=$(readlink "${so_link}")
					# If symlink target is absolute and starts with /lib/, make it relative
					if [[ "${link_target}" == /lib/* ]]; then
						ln -sf "../${link_target}" "${so_link}" 2>/dev/null || true
					fi
				fi
			done
		fi
	done

	# The GCC CRT files (crtbeginS.o, crtendS.o, libgcc.a, ...) come from the
	# Debian libgcc-<ver>-dev package extracted earlier into
	#   ${sysroot_dir}/usr/lib/gcc/${gnu_arch}/${gcc_version}/
	# This is the ONLY supported source: a host-installed cross-gcc (e.g.
	# aarch64-linux-gnu-gcc 16) reports a version that may not exist in the
	# target Debian suite and would inject incompatible CRT objects, so we
	# never copy from it. Verify the Debian-extracted CRT is present instead.
	local gcc_install_dir="${sysroot_dir}/usr/lib/gcc/${gnu_arch}/${gcc_version}"
	if [[ ! -d "${gcc_install_dir}" ]]; then
		# Fall back to a recursive search under <sysroot>/usr/lib/gcc
		gcc_install_dir=$(find "${sysroot_dir}/usr/lib/gcc" -name "crtbeginS.o" -path "*/gcc/*" 2>/dev/null | head -1 | xargs dirname 2>/dev/null || echo "")
	fi

	if [[ -z "${gcc_install_dir}" || ! -d "${gcc_install_dir}" ]]; then
		log_error "GCC CRT files (crtbeginS.o) not found in sysroot"
		log_error "Ensure libgcc-${gcc_version}-dev was extracted for ${target}"
		return 1
	fi

	# Install libstdc++ in the correct GCC version directory
	local gcc_std_dir="${sysroot_dir}/usr/lib/gcc/${gnu_arch}/${gcc_version}"
	if [[ -d "${gcc_std_dir}" ]]; then
		# Move .a files from /usr/lib/ to GCC version directory if present
		if [[ -f "${sysroot_dir}/usr/lib/libstdc++.a" ]]; then
			cp "${sysroot_dir}/usr/lib/libstdc++.a" "${gcc_std_dir}/"
		fi
		if [[ -f "${sysroot_dir}/usr/lib/libsupc++.a" ]]; then
			cp "${sysroot_dir}/usr/lib/libsupc++.a" "${gcc_std_dir}/"
		fi
	fi

	log_info "Sysroot prepared: ${sysroot_dir}"
	log_info "  Sysroot lib dir: ${sysroot_dir}/lib"
	log_info "  GCC install dir: ${gcc_install_dir}"
}

main() {
	local output_dir="${DEFAULT_OUTPUT_DIR}"
	local suite=""
	local targets=()
	local prepare_all=false

	while [[ $# -gt 0 ]]; do
		case "$1" in
			-o | --output-dir)
				output_dir="$2"
				shift 2
				;;
			-s | --suite)
				suite="$2"
				shift 2
				;;
			--all)
				prepare_all=true
				shift
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
				targets+=("$1")
				shift
				;;
		esac
	done

	if [[ "${prepare_all}" == "true" ]]; then
		targets=("${SUPPORTED_TARGETS[@]}")
	fi

	if [[ ${#targets[@]} -eq 0 ]]; then
		log_error "No target specified"
		usage >&2
		exit 1
	fi

	for target in "${targets[@]}"; do
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

		prepare_sysroot "${target}" "${output_dir}" "${suite}"
	done

	log_info "All GNU sysroots prepared in: ${output_dir}"
}

main "$@"
