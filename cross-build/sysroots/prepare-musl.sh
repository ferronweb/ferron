#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEFAULT_OUTPUT_DIR="${SCRIPT_DIR}/../sysroots/prepared"

ALPINE_MIRROR="https://dl-cdn.alpinelinux.org/alpine"
ALPINE_VERSION="3.24"
ALPINE_REPO="main"

SUPPORTED_TARGETS=(
	"x86_64-unknown-linux-musl"
	"i686-unknown-linux-musl"
	"aarch64-unknown-linux-musl"
	"armv7-unknown-linux-musleabihf"
	"riscv64gc-unknown-linux-musl"
)

usage() {
	cat <<EOF
Usage: $(basename "$0") [OPTIONS] <TARGET>

Create an Alpine-based sysroot with musl libc and LLVM libc++.

Targets:
$(printf '  %s\n' "${SUPPORTED_TARGETS[@]}")

Options:
  -o, --output-dir DIR    Output directory (default: ${DEFAULT_OUTPUT_DIR})
  -v, --version VER       Alpine version (default: ${ALPINE_VERSION})
  -h, --help              Show this help message

Examples:
  $(basename "$0") x86_64-unknown-linux-musl
  $(basename "$0") -o /opt/sysroots aarch64-unknown-linux-musl
  $(basename "$0") --all
EOF
}

log_info() {
	echo "[INFO] $*"
}

log_error() {
	echo "[ERROR] $*" >&2
}

target_to_alpine_arch() {
	local target="$1"
	case "$target" in
		x86_64-unknown-linux-musl) echo "x86_64" ;;
		i686-unknown-linux-musl) echo "x86" ;;
		aarch64-unknown-linux-musl) echo "aarch64" ;;
		armv7-unknown-linux-musleabihf) echo "armv7" ;;
		riscv64gc-unknown-linux-musl) echo "riscv64" ;;
		*) log_error "Unknown musl target: $target"; return 1 ;;
	esac
}

host_arch() {
	case "$(uname -m)" in
		x86_64) echo "x86_64" ;;
		aarch64) echo "aarch64" ;;
		armv7*) echo "armv7" ;;
		riscv64) echo "riscv64" ;;
		*) echo "$(uname -m)" ;;
	esac
}

host_arch_to_alpine() {
	local arch
	arch=$(host_arch)
	case "$arch" in
		x86_64) echo "x86_64" ;;
		aarch64) echo "aarch64" ;;
		armv7) echo "armv7" ;;
		riscv64) echo "riscv64" ;;
		*) echo "$arch" ;;
	esac
}

# Download and extract an Alpine APK package into a directory
extract_apk() {
	local apk_url="$1"
	local dest_dir="$2"
	local tmp_dir

	tmp_dir=$(mktemp -d)
	local apk_file="${tmp_dir}/$(basename "${apk_url}")"

	log_info "  Downloading: ${apk_url}"
	if ! curl -fsSL -o "${apk_file}" "${apk_url}"; then
		rm -rf "${tmp_dir}"
		return 1
	fi

	# APK files are just .tar.gz
	mkdir -p "${dest_dir}"
	tar -xzf "${apk_file}" -C "${dest_dir}"
	rm -rf "${tmp_dir}"
}

# Get the version of an Alpine package from the APK index
get_apk_version() {
	local package_name="$1"
	local alpine_arch="$2"
	local alpine_version="$3"
	local index_url="${ALPINE_MIRROR}/v${alpine_version}/${ALPINE_REPO}/${alpine_arch}/APKINDEX.tar.gz"
	local tmp_dir

	tmp_dir=$(mktemp -d)
	local index_file="${tmp_dir}/APKINDEX.tar.gz"

	if ! curl -fsSL -o "${index_file}" "${index_url}"; then
		rm -rf "${tmp_dir}"
		return 1
	fi

	tar -xzf "${index_file}" -C "${tmp_dir}"

	# Parse the APKINDEX to find the package version
	# Each package block is separated by blank lines, with P: for name and V: for version
	local version
	version=$(awk -v pkg="${package_name}" '
		/^P:/ { current_pkg = substr($0, 3); found = 0 }
		current_pkg == pkg { found = 1 }
		/^V:/ && found { print substr($0, 3); exit }
	' "${tmp_dir}/APKINDEX")

	rm -rf "${tmp_dir}"

	if [[ -z "${version}" ]]; then
		return 1
	fi
	echo "${version}"
}

prepare_sysroot() {
	local target="$1"
	local output_dir="$2"
	local alpine_version="$3"
	local alpine_arch
	local sysroot_dir
	local host_arch_name

	alpine_arch=$(target_to_alpine_arch "$target")
	host_arch_name=$(host_arch_to_alpine)
	sysroot_dir="${output_dir}/musl-${target}"

	log_info "Preparing musl sysroot for ${target}"
	log_info "  Alpine arch: ${alpine_arch}"
	log_info "  Alpine version: ${alpine_version}"
	log_info "  Output: ${sysroot_dir}"

	if [[ -d "${sysroot_dir}" ]]; then
		log_info "Sysroot already exists, removing: ${sysroot_dir}"
		rm -rf "${sysroot_dir}"
	fi

	mkdir -p "${sysroot_dir}"

	# Get package versions
	log_info "Fetching Alpine package versions"
	local musl_version musl_dev_version libcxx_static_version libcxx_dev_version linux_headers_version libgcc_static_version
	musl_version=$(get_apk_version "musl" "${alpine_arch}" "${alpine_version}")
	musl_dev_version=$(get_apk_version "musl-dev" "${alpine_arch}" "${alpine_version}")
	libcxx_static_version=$(get_apk_version "libc++-static" "${alpine_arch}" "${alpine_version}")
	libcxx_dev_version=$(get_apk_version "libc++-dev" "${alpine_arch}" "${alpine_version}")
	linux_headers_version=$(get_apk_version "linux-headers" "${alpine_arch}" "${alpine_version}")
	libgcc_static_version=$(get_apk_version "libgcc-static" "${alpine_arch}" "${alpine_version}")
	gcc_version=$(get_apk_version "gcc" "${alpine_arch}" "${alpine_version}")

	if [[ -z "${musl_version}" ]]; then
		log_error "Could not find musl version for ${alpine_arch}"
		return 1
	fi
	if [[ -z "${musl_dev_version}" ]]; then
		log_error "Could not find musl-dev version for ${alpine_arch}"
		return 1
	fi
	if [[ -z "${gcc_version}" ]]; then
		log_error "Could not find gcc version for ${alpine_arch}"
		return 1
	fi
	if [[ -z "${libcxx_static_version}" ]]; then
		log_error "Could not find libc++-static version for ${alpine_arch}"
		return 1
	fi
	if [[ -z "${libcxx_dev_version}" ]]; then
		log_error "Could not find libc++-dev version for ${alpine_arch}"
		return 1
	fi
	if [[ -z "${linux_headers_version}" ]]; then
		log_error "Could not find linux-headers version for ${alpine_arch}"
		return 1
	fi
	if [[ -z "${libgcc_static_version}" ]]; then
		log_error "Could not find libgcc-static version for ${alpine_arch} (Alpine ${alpine_version}?)"
		return 1
	fi

	log_info "  musl: ${musl_version}"
	log_info "  musl-dev: ${musl_dev_version}"
	log_info "  gcc: ${gcc_version}"
	log_info "  libc++-static: ${libcxx_static_version}"
	log_info "  libc++-dev: ${libcxx_dev_version}"
	log_info "  linux-headers: ${linux_headers_version}"
	log_info "  libgcc-static: ${libgcc_static_version}"

	local apk_base="${ALPINE_MIRROR}/v${alpine_version}/${ALPINE_REPO}/${alpine_arch}"

	# Download and extract musl (provides dynamic linker and libc.so)
	local tmp_dir
	tmp_dir=$(mktemp -d)

	log_info "Downloading and extracting musl"
	extract_apk "${apk_base}/musl-${musl_version}.apk" "${tmp_dir}/musl"
	mkdir -p "${sysroot_dir}/lib"
	# Copy dynamic linker and libc.so (musl's libc.so IS the dynamic linker)
	if [[ -d "${tmp_dir}/musl/lib" ]]; then
		for f in "${tmp_dir}/musl/lib/ld-musl-"*.so* "${tmp_dir}/musl/lib/libc.musl-"*.so*; do
			if [[ -e "$f" ]]; then
				cp -a "$f" "${sysroot_dir}/lib/"
			fi
		done
	fi
	# Set up libc.so symlink pointing to dynamic linker
	rm -f "${sysroot_dir}/lib/libc.so"
	if [[ -e "${sysroot_dir}/lib/ld-musl-x86_64.so.1" ]]; then
		ln -s "ld-musl-x86_64.so.1" "${sysroot_dir}/lib/libc.so"
	fi
	rm -rf "${tmp_dir}/musl"

	# Download and extract musl-dev (provides musl headers and musl-gcc.specs)
	log_info "Downloading and extracting musl-dev"
	extract_apk "${apk_base}/musl-dev-${musl_dev_version}.apk" "${tmp_dir}/musl-dev"
	mv "${tmp_dir}/musl-dev/usr/include" "${sysroot_dir}/include"
	mkdir -p "${sysroot_dir}/lib"
	if [[ -d "${tmp_dir}/musl-dev/usr/lib" ]]; then
		# Copy CRT files and other libs, but skip libc.so symlink (we handle it separately)
		for f in "${tmp_dir}/musl-dev/usr/lib/"*; do
			name=$(basename "$f")
			if [[ "$name" != "libc.so" ]]; then
				cp -a "$f" "${sysroot_dir}/lib/" 2>/dev/null || true
			fi
		done
	fi
	rm -rf "${tmp_dir}/musl-dev"

	# Download and extract libc++-static (provides libc++ static libraries)
	log_info "Downloading and extracting libc++-static"
	extract_apk "${apk_base}/libc++-static-${libcxx_static_version}.apk" "${tmp_dir}/libcxx-static"
	mkdir -p "${sysroot_dir}/lib"
	if [[ -d "${tmp_dir}/libcxx-static/usr/lib" ]]; then
		cp -a "${tmp_dir}/libcxx-static/usr/lib/"*.a "${sysroot_dir}/lib/" 2>/dev/null || true
	fi
	rm -rf "${tmp_dir}/libcxx-static"

	# Download and extract libc++-dev (provides libc++ headers)
	log_info "Downloading and extracting libc++-dev"
	extract_apk "${apk_base}/libc++-dev-${libcxx_dev_version}.apk" "${tmp_dir}/libcxx-dev"
	mkdir -p "${sysroot_dir}/include/c++/v1"
	if [[ -d "${tmp_dir}/libcxx-dev/usr/include/c++/v1" ]]; then
		cp -a "${tmp_dir}/libcxx-dev/usr/include/c++/v1/"* "${sysroot_dir}/include/c++/v1/"
	fi
	rm -rf "${tmp_dir}/libcxx-dev"

	# Download and extract linux-headers (provides asm/unistd.h and other kernel headers)
	log_info "Downloading and extracting linux-headers"
	extract_apk "${apk_base}/linux-headers-${linux_headers_version}.apk" "${tmp_dir}/linux-headers"
	if [[ -d "${tmp_dir}/linux-headers/usr/include" ]]; then
		# linux-headers puts asm/, asm-generic/, linux/, etc. in usr/include
		for dir in asm asm-generic linux; do
			if [[ -d "${tmp_dir}/linux-headers/usr/include/${dir}" ]]; then
				mkdir -p "${sysroot_dir}/include/${dir}"
				cp -a "${tmp_dir}/linux-headers/usr/include/${dir}/"* "${sysroot_dir}/include/${dir}/" 2>/dev/null || true
			fi
		done
	fi
	rm -rf "${tmp_dir}/linux-headers"

	rm -rf "${tmp_dir}"

	# Provide GCC CRT startup objects (crtbeginS.o/crtendS.o) and libgcc.a for the
	# TARGET architecture. These come from Alpine's libgcc-static package (built for
	# musl), NOT from the host's native GCC. This keeps the musl toolchain fully
	# clang/lld-based with no host GCC dependency.
	log_info "Fetching target GCC runtime (libgcc-static) from Alpine"
	local libgcc_dir="${tmp_dir}/libgcc-static"
	extract_apk "${apk_base}/libgcc-static-${libgcc_static_version}.apk" "${libgcc_dir}"
	# The package lays files out as usr/lib/gcc/<triplet>/<ver>/{crtbeginS.o,crtendS.o,libgcc.a}
	local gcc_obj_src
	gcc_obj_src=$(find "${libgcc_dir}" -name 'crtbeginS.o' 2>/dev/null | head -1)
	if [[ -n "${gcc_obj_src}" ]]; then
		local gcc_obj_root
		gcc_obj_root=$(dirname "${gcc_obj_src}")
		for obj in crtbeginS.o crtendS.o; do
			if [[ -f "${gcc_obj_root}/${obj}" ]]; then
				cp "${gcc_obj_root}/${obj}" "${sysroot_dir}/lib/"
				log_info "  Copied ${obj}"
			fi
		done
		if [[ -f "${gcc_obj_root}/libgcc.a" ]]; then
			cp "${gcc_obj_root}/libgcc.a" "${sysroot_dir}/lib/"
			log_info "  Copied libgcc.a"
		fi
	else
		log_error "libgcc-static did not contain crtbeginS.o (unexpected layout)"
		return 1
	fi
	rm -rf "${tmp_dir}"

    log_info "Fetching GCC (gcc) from Alpine"
	local gcc_dir="${tmp_dir}/gcc"
	extract_apk "${apk_base}/gcc-${gcc_version}.apk" "${gcc_dir}"
	# The package lays files out as usr/lib/gcc/<triplet>/<ver>/{crtbeginT.o,crtend.o}
	local gcc_obj_src
	gcc_obj_src=$(find "${gcc_dir}" -name 'crtbeginT.o' 2>/dev/null | head -1)
	if [[ -n "${gcc_obj_src}" ]]; then
		local gcc_obj_root
		gcc_obj_root=$(dirname "${gcc_obj_src}")
		for obj in crtbeginT.o crtend.o; do
			if [[ -f "${gcc_obj_root}/${obj}" ]]; then
				cp "${gcc_obj_root}/${obj}" "${sysroot_dir}/lib/"
				log_info "  Copied ${obj}"
			fi
		done
	else
		log_error "gcc did not contain crtbeginT.o (unexpected layout)"
		return 1
	fi
	# Install libatomic.a from gcc
	if [[ -f "${gcc_dir}/lib/libatomic.a" ]]; then
		cp "${gcc_dir}/lib/libatomic.a" "${sysroot_dir}/lib/"
		log_info "  Copied libatomic.a"
	fi
	rm -rf "${tmp_dir}"

	log_info "musl sysroot prepared successfully: ${sysroot_dir}"
	log_info "  Headers: ${sysroot_dir}/include"
	log_info "  Libraries: ${sysroot_dir}/lib"
}

main() {
	local output_dir="${DEFAULT_OUTPUT_DIR}"
	local alpine_version="${ALPINE_VERSION}"
	local targets=()
	local prepare_all=false

	while [[ $# -gt 0 ]]; do
		case "$1" in
			-o | --output-dir)
				output_dir="$2"
				shift 2
				;;
			-v | --version)
				alpine_version="$2"
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

		prepare_sysroot "${target}" "${output_dir}" "${alpine_version}"
	done

	log_info "All musl sysroots prepared in: ${output_dir}"
}

main "$@"
