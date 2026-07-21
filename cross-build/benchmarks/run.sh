#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

SUPPORTED_TARGETS=(
	"x86_64-unknown-linux-gnu"
	"i686-unknown-linux-gnu"
	"aarch64-unknown-linux-gnu"
	"armv7-unknown-linux-gnueabihf"
	"riscv64gc-unknown-linux-gnu"
	"s390x-unknown-linux-gnu"
	"powerpc64le-unknown-linux-gnu"
	"x86_64-unknown-linux-musl"
	"i686-unknown-linux-musl"
	"aarch64-unknown-linux-musl"
	"armv7-unknown-linux-musleabihf"
	"riscv64gc-unknown-linux-musl"
)

usage() {
	cat <<EOF
Usage: $(basename "$0") <BINARY> <TARGET> [OPTIONS]

Run PGO training benchmarks against a Ferron binary.

Arguments:
  BINARY              Path to the ferron binary
  TARGET              Rust target triple

Options:
  -d, --duration SEC  Benchmark duration per scenario (default: 30)
  -p, --port PORT     Base port for servers (default: 18080)
  --pgo-data-dir DIR  Directory for .profraw files
  -h, --help          Show this help message

Scenarios:
  1. Small static files (1KB) via wrk (HTTP/1.1)
  2. Large static files (1MB) via wrk (HTTP/1.1)
  3. Reverse proxy via wrk (HTTP/1.1)
  4. Reverse proxy via h2load (HTTP/2 + TLS)
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
	echo "----------------------------------------"
	echo "  $*"
	echo "----------------------------------------"
	echo ""
}

cleanup() {
	log_info "Cleaning up benchmark processes..."
	if [[ -n "${FERRON_PID:-}" ]] && kill -0 "${FERRON_PID}" 2>/dev/null; then
		kill -INT "${FERRON_PID}" 2>/dev/null || true
		wait "${FERRON_PID}" 2>/dev/null || true
	fi
	if [[ -n "${BACKEND_PID:-}" ]] && kill -0 "${BACKEND_PID}" 2>/dev/null; then
		kill "${BACKEND_PID}" 2>/dev/null || true
		wait "${BACKEND_PID}" 2>/dev/null || true
	fi
	if [[ -n "${WORK_DIR:-}" ]] && [[ -d "${WORK_DIR}" ]]; then
		rm -rf "${WORK_DIR}"
	fi
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

target_to_arch() {
	echo "${1}" | cut -d'-' -f1
}

obtain_sysroot_ld_library_paths() {
	# Also, interpret "include /path/to/ld.so.conf/*" as a path to include
	local sysroot="$1"
	sysroot="${sysroot%/}" # Remove trailing "/" from sysroot
	local ld_library_paths=()
    local ld_so_conf_files
    if [[ -f "${sysroot}/etc/ld.so.conf" ]]; then
        ld_so_conf_files=("${sysroot}/etc/ld.so.conf")
    else
        ld_so_conf_files=($(find "${sysroot}/etc/ld.so.conf.d/" -type f))
    fi
    local ld_so_conf_file="${ld_so_conf_files[0]:-}"
    ld_so_conf_files=(${ld_so_conf_files[@]:1})
    while [[ -n "${ld_so_conf_file}" ]]; do
        local sanitized_ld_so_conf
        sanitized_ld_so_conf=$(cat "${ld_so_conf_file}" | grep -vE '^(#.*)?$')
        # Extract includes
        local ld_so_conf_includes
        local ld_so_conf_includes_str
        ld_so_conf_includes_str=$(echo "${sanitized_ld_so_conf}" | grep -E '^include' | sed -E 's/^include\s*//' | xargs)
        IFS=' ' read -r -a ld_so_conf_includes <<< "${ld_so_conf_includes_str}"
        ld_so_conf_includes=( "${ld_so_conf_includes[@]/#/${sysroot}}" ) # Prepend sysroot to include paths
        # Extract non-include lines
        local ld_so_conf_paths
        local ld_so_conf_paths_str
        ld_so_conf_paths_str=$(echo "${sanitized_ld_so_conf}" | grep -vE '^include' | xargs)
        IFS=' ' read -r -a ld_so_conf_paths <<< "${ld_so_conf_paths_str}"
        ld_so_conf_paths=( "${ld_so_conf_paths[@]/#/${sysroot}}" ) # Prepend sysroot to library paths

        ld_library_paths+=(${ld_so_conf_paths[@]})
        ld_so_conf_files+=(${ld_so_conf_includes[@]})
        ld_so_conf_file="${ld_so_conf_files[0]:-}"
        ld_so_conf_files=(${ld_so_conf_files[@]:1})
    done
    echo "$(IFS=':'; echo -n "${ld_library_paths[*]}")"
}

generate_self_signed_cert() {
	local cert_dir="$1"
	mkdir -p "${cert_dir}"

	if command -v openssl &>/dev/null; then
		openssl req -x509 -newkey rsa:2048 -keyout "${cert_dir}/key.pem" \
			-out "${cert_dir}/cert.pem" -days 1 -nodes \
			-subj "/CN=localhost" 2>/dev/null
		log_info "Generated self-signed TLS certificate"
	else
		log_error "openssl not found. Cannot generate TLS certificate for HTTP/2 benchmarks."
		log_error "Install openssl or skip HTTP/2 scenarios."
		return 1
	fi
}

create_bench_files() {
	local bench_dir="$1"

	# Create 1KB static file
	mkdir -p "${bench_dir}/static"
	dd if=/dev/urandom bs=1024 count=1 2>/dev/null | base64 > "${bench_dir}/static/1k.txt.tmp"
 head -c 1024 "${bench_dir}/static/1k.txt.tmp" > "${bench_dir}/static/1k.txt"
	rm -f "${bench_dir}/static/1k.txt.tmp"

	# Create 1MB static file
	dd if=/dev/urandom bs=1024 count=1024 2>/dev/null | base64 > "${bench_dir}/static/1m.txt.tmp"
 head -c 1048576 "${bench_dir}/static/1m.txt.tmp" > "${bench_dir}/static/1m.txt"
	rm -f "${bench_dir}/static/1m.txt.tmp"

	log_info "Created benchmark files in ${bench_dir}/static/"
}

create_ferron_config() {
	local config_file="$1"
	local bench_dir="$2"
	local ferron_port="$3"
	local tls_port="$4"
	local cert_dir="$5"
	local backend_port="$6"
	local cross_compiled="$7"

	if [[ "${cross_compiled}" == "true" ]]; then
		# Disable io_uring and use static file mode for cross-compiled binaries
		# (tokio file I/O crashes under QEMU under high concurrency)
		cat > "${config_file}" <<EOF
{
    log /dev/null
    error_log /dev/null
    runtime {
        io_uring false
    }
}

*:${ferron_port} {
    root ${bench_dir}
}
EOF
	else
		cat > "${config_file}" <<EOF
{
    log /dev/null
    error_log /dev/null
}

*:${ferron_port} {
    root ${bench_dir}
}

*:${tls_port} {
    tls ${cert_dir}/cert.pem ${cert_dir}/key.pem
    root ${bench_dir}
}
EOF
	fi

	log_info "Created ferron config: ${config_file}"
}

start_backend_server() {
	local bench_dir="$1"
	local port="$2"

	# Use Python's built-in HTTP server as a simple backend
	if command -v python3 &>/dev/null; then
		(cd "${bench_dir}" && python3 -m http.server "${port}" &>/dev/null) &
		BACKEND_PID=$!
		log_info "Started Python HTTP backend on port ${port} (PID: ${BACKEND_PID})"
	elif command -v python &>/dev/null; then
		(cd "${bench_dir}" && python -m SimpleHTTPServer "${port}" &>/dev/null) &
		BACKEND_PID=$!
		log_info "Started Python HTTP backend on port ${port} (PID: ${BACKEND_PID})"
	else
		log_error "Python not found. Required for the reverse proxy backend server."
		return 1
	fi
}

run_wrk() {
	local url="$1"
	local lua_script="$2"
	local duration="$3"
	local label="$4"

	log_info "Running wrk: ${label}"
	log_info "  URL: ${url}"
	log_info "  Duration: ${duration}s"

	if [[ -f "${lua_script}" ]]; then
		wrk -t4 -c100 -d"${duration}s" -s "${lua_script}" "${url}" 2>&1 | tee -a "${BENCH_LOG}"
	else
		wrk -t4 -c100 -d"${duration}s" "${url}" 2>&1 | tee -a "${BENCH_LOG}"
	fi

	echo "" >> "${BENCH_LOG}"
}

run_h2load() {
	local url="$1"
	local duration="$2"
	local label="$3"
	local connections="${4:-100}"
	local threads="${5:-4}"

	# Calculate number of requests (high number to ensure duration-based run)
	local requests=$((connections * 10 * duration))

	log_info "Running h2load: ${label}"
	log_info "  URL: ${url}"
	log_info "  Connections: ${connections}, Threads: ${threads}"
	log_info "  Duration: ~${duration}s"

	h2load \
		-n "${requests}" \
		-c "${connections}" \
		-t "${threads}" \
		--connection-active-timeout "${duration}" \
		--connection-inactivity-timeout "${duration}" \
		"${url}" 2>&1 | tee -a "${BENCH_LOG}"

	echo "" >> "${BENCH_LOG}"
}

main() {
	local binary=""
	local target=""
	local duration=30
	local base_port=18080
	local pgo_data_dir=""
	local ld_library_path="${LD_LIBRARY_PATH:-}"

	while [[ $# -gt 0 ]]; do
		case "$1" in
			-d | --duration)
				duration="$2"
				shift 2
				;;
			-p | --port)
				base_port="$2"
				shift 2
				;;
			--pgo-data-dir)
				pgo_data_dir="$2"
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
				if [[ -z "${binary}" ]]; then
					binary="$1"
				elif [[ -z "${target}" ]]; then
					target="$1"
				fi
				shift
				;;
		esac
	done

	if [[ -z "${binary}" ]] || [[ -z "${target}" ]]; then
		log_error "BINARY and TARGET are required"
		usage >&2
		exit 1
	fi

	if [[ ! -x "${binary}" ]]; then
		log_error "Binary not found or not executable: ${binary}"
		exit 1
	fi

	# Check required tools (only needed for native builds)
	local host
	host=$(host_arch)
	local target_arch
	target_arch=$(target_to_arch "${target}")

	if [[ "${host}" == "${target_arch}" ]]; then
		local missing=()
		for cmd in wrk h2load; do
			if ! command -v "${cmd}" &>/dev/null; then
				missing+=("${cmd}")
			fi
		done

		if [[ ${#missing[@]} -gt 0 ]]; then
			log_error "Missing benchmark tools: ${missing[*]}"
			log_error ""
			log_error "Install with:"
			log_error "  Arch:       sudo pacman -S wrk h2load"
			log_error "  Debian/Ubuntu: sudo apt install wrk (h2load from nghttp2)"
			log_error "  From source: https://github.com/wg/wrk, https://github.com/nghttp2/nghttp2"
			exit 1
		fi
	fi

	# Check if running cross-compiled binary
	local host
	host=$(host_arch)
	local target_arch
	target_arch=$(target_to_arch "${target}")

	if [[ "${host}" != "${target_arch}" ]]; then
		local qemu_binary
		qemu_binary=$(target_to_qemu_binary "${target}")
		if [[ -z "${qemu_binary}" ]]; then
			log_error "No QEMU binary for target: ${target}"
			exit 1
		fi
		if ! command -v "${qemu_binary}" &>/dev/null; then
			log_error "QEMU user-static not found: ${qemu_binary}"
			log_error "Install with: sudo pacman -S qemu-user-static qemu-user-static-binfmt"
			exit 1
		fi
		binary="${qemu_binary} ${binary}"
	fi

	# Set LD_LIBRARY_PATH
	if [[ -d "$QEMU_LD_PREFIX" ]]; then
	    # Obtain paths from etc/ld.so.conf and etc/ld.so.conf.d/*, if available
		# Ignore empty lines and lines starting from "#" (comments)
		# Also, trim lines
		local sysroot_ld_library_paths
		sysroot_ld_library_paths=$(obtain_sysroot_ld_library_paths $QEMU_LD_PREFIX)
		if [[ -z "${sysroot_ld_library_paths}" ]]; then
			ld_library_path="$QEMU_LD_PREFIX/usr/local/lib:$QEMU_LD_PREFIX/lib:$QEMU_LD_PREFIX/usr/lib"
		else
		    ld_library_path="${sysroot_ld_library_paths}:$QEMU_LD_PREFIX/usr/local/lib:$QEMU_LD_PREFIX/lib:$QEMU_LD_PREFIX/usr/lib"
		fi
		log_info "LD_LIBRARY_PATH: ${ld_library_path}"
	fi

	# Setup temporary directory
	local work_dir
	work_dir=$(mktemp -d)
	trap cleanup EXIT
	local bench_dir="${work_dir}/bench"
	local cert_dir="${work_dir}/certs"
	local ferron_port="${base_port}"
	local backend_port=$((base_port + 1))
	local tls_port=$((base_port + 2))

	mkdir -p "${bench_dir}" "${cert_dir}"

	BENCH_LOG="${work_dir}/bench.log"
	touch "${BENCH_LOG}"

	# Set PGO data directory
	if [[ -z "${pgo_data_dir}" ]]; then
		pgo_data_dir="${work_dir}/pgo-data"
	fi
	mkdir -p "${pgo_data_dir}"
	export LLVM_PROFILE_FILE="${pgo_data_dir}/ferron-%p-%m.profraw"

	log_step "Preparing benchmark environment"

	# Generate TLS certificate
	generate_self_signed_cert "${cert_dir}"

	# Create benchmark files
	create_bench_files "${bench_dir}"

	# Create ferron config
	local config_file="${work_dir}/ferron.conf"
	local cross_compiled="false"
	if [[ "${host}" != "${target_arch}" ]]; then
		cross_compiled="true"
	fi
	create_ferron_config "${config_file}" "${bench_dir}" "${ferron_port}" "${tls_port}" "${cert_dir}" "${backend_port}" "${cross_compiled}"

	# Start backend server
	start_backend_server "${bench_dir}" "${backend_port}"

	# Wait for backend to start
	sleep 1

	# Start ferron
	log_step "Starting Ferron"

	# shellcheck disable=SC2086
	if [[ -z "${ld_library_path}" ]]; then
		${binary} run -c "${config_file}" &
		FERRON_PID=$!
	else
		LD_LIBRARY_PATH="${ld_library_path}" ${binary} run -c "${config_file}" &
		FERRON_PID=$!
	fi
	log_info "Ferron started (PID: ${FERRON_PID})"

	# Wait for ferron to start listening
	sleep 2

	# Verify ferron is running
	if ! kill -0 "${FERRON_PID}" 2>/dev/null; then
		log_error "Ferron failed to start"
		log_error "Check the binary and configuration"
		exit 1
	fi

	log_step "Running PGO training benchmarks"

	local lua_dir="${SCRIPT_DIR}/scenarios"

	if [[ "${cross_compiled}" == "true" ]]; then
		# Use curl loops for cross-compiled binaries — wrk's high concurrency
		# triggers tokio::fs crashes under QEMU
		log_info "Running curl-based PGO training (cross-compiled target)"

		log_info "  Scenario: Small static files (1KB) - HTTP/1.1"
		for i in $(seq 1 200); do
			curl -s -o /dev/null "http://127.0.0.1:${ferron_port}/static/1k.txt" 2>/dev/null
		done

		log_info "  Scenario: Large static files (1MB) - HTTP/1.1"
		for i in $(seq 1 20); do
			curl -s -o /dev/null "http://127.0.0.1:${ferron_port}/static/1m.txt" 2>/dev/null
		done
	else
		# Scenario 1: Small static files (HTTP/1.1)
		run_wrk \
			"http://127.0.0.1:${ferron_port}/static/1k.txt" \
			"${lua_dir}/static-small.lua" \
			"${duration}" \
			"Small static files (1KB) - HTTP/1.1"

		# Scenario 2: Large static files (HTTP/1.1)
		run_wrk \
			"http://127.0.0.1:${ferron_port}/static/1m.txt" \
			"${lua_dir}/static-large.lua" \
			"${duration}" \
			"Large static files (1MB) - HTTP/1.1"

		# Scenario 3: Reverse proxy (HTTP/1.1)
		run_wrk \
			"http://127.0.0.1:${ferron_port}/static/1k.txt" \
			"${lua_dir}/proxy-http1.lua" \
			"${duration}" \
			"Reverse proxy - HTTP/1.1"

		# Scenario 4: Reverse proxy (HTTP/2 + TLS)
		run_h2load \
			"https://127.0.0.1:${tls_port}/static/1k.txt" \
			"${duration}" \
			"Reverse proxy - HTTP/2 + TLS"
	fi

	log_step "Benchmark complete"

	# Stop ferron
	if kill -0 "${FERRON_PID}" 2>/dev/null; then
		kill -INT "${FERRON_PID}" 2>/dev/null || true
		wait "${FERRON_PID}" 2>/dev/null || true
	fi

	# Stop backend
	if [[ -n "${BACKEND_PID:-}" ]] && kill -0 "${BACKEND_PID}" 2>/dev/null; then
		kill "${BACKEND_PID}" 2>/dev/null || true
		wait "${BACKEND_PID}" 2>/dev/null || true
	fi

	# Show PGO data
	local profraw_count
	profraw_count=$(find "${pgo_data_dir}" -name "*.profraw" 2>/dev/null | wc -l)

	log_info "PGO data directory: ${pgo_data_dir}"
	log_info "Profile files generated: ${profraw_count}"

	if [[ "${profraw_count}" -gt 0 ]]; then
		log_info "Profile files:"
		ls -lh "${pgo_data_dir}"/*.profraw 2>/dev/null | awk '{print "  " $NF " (" $5 ")"}'
	fi

	log_info "Benchmark log: ${BENCH_LOG}"

	# Cleanup
	rm -rf "${work_dir}"
}

main "$@"
