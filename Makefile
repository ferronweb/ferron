FERRON_VERSION_CARGO = $(shell cat ferron/Cargo.toml | grep -E '^version' | sed -E 's|.*"([0-9a-zA-Z.+-]+)"$$|\1|g')
FERRON_VERSION_GIT = $(shell git tag --sort=-committerdate | head -n 1 | sed s/[^0-9a-zA-Z.+-]//g)
ifdef FERRON_VERSION_CARGO
	FERRON_VERSION = $(FERRON_VERSION_CARGO)
else
	FERRON_VERSION = $(FERRON_VERSION_GIT)
endif
HOST_TARGET_TRIPLE = $(shell rustc -vV | sed -n 's|host: ||p')

CARGO_FINAL_EXTRA_ARGS_ENV := $(CARGO_FINAL_EXTRA_ARGS)
ifdef TARGET
	CARGO_FINAL_EXTRA_ARGS = --target $(TARGET) $(CARGO_FINAL_EXTRA_ARGS_ENV)
	CARGO_TARGET_ROOT = target/$(TARGET)
	DEST_TARGET_TRIPLE = $(TARGET)
	BUILD_RELEASE = build-release-$(TARGET)
else
	CARGO_FINAL_EXTRA_ARGS = $(CARGO_FINAL_EXTRA_ARGS_ENV)
	CARGO_TARGET_ROOT = target
	DEST_TARGET_TRIPLE = $(HOST_TARGET_TRIPLE)
	BUILD_RELEASE = build-release
endif

ifdef NO_MONOIO
    CARGO_FINAL_EXTRA_ARGS_NO_MONOIO_ENV := $(CARGO_FINAL_EXTRA_ARGS)
    CARGO_FINAL_EXTRA_ARGS = --no-default-features -F ferron/runtime-tokio $(CARGO_FINAL_EXTRA_ARGS_NO_MONOIO_ENV)
endif

ifndef CARGO_FINAL
	CARGO_FINAL = cargo
endif

.PHONY: build

smoketest-dev: build-dev
	FERRON="$(PWD)/$(CARGO_TARGET_ROOT)/debug/ferron" smoketest/smoketest.sh

smoketest: build
	FERRON="$(PWD)/$(CARGO_TARGET_ROOT)/release/ferron" smoketest/smoketest.sh

run: build
	if ! [ -f "ferron.kdl" ]; then cp ferron-test.kdl ferron.kdl; fi
	$(CARGO_TARGET_ROOT)/release/ferron

run-dev: build-dev
	if ! [ -f "ferron.kdl" ]; then cp ferron-test.kdl ferron.kdl; fi
	$(CARGO_TARGET_ROOT)/debug/ferron

build: prepare-build fix-conflicts
	cd build-workspace && RUST_LIBC_UNSTABLE_MUSL_V1_2_3=1 $(CARGO_FINAL) build --target-dir ../target -r $(CARGO_FINAL_EXTRA_ARGS)

build-dev: prepare-build fix-conflicts
	cd build-workspace && RUST_LIBC_UNSTABLE_MUSL_V1_2_3=1 $(CARGO_FINAL) build --target-dir ../target $(CARGO_FINAL_EXTRA_ARGS)

prepare-build:
	cargo run --manifest-path build-prepare/Cargo.toml

fix-conflicts:
	@ cd build-workspace && \
	while [ "$$OLD_CONFLICTING_PACKAGES" != "$$CONFLICTING_PACKAGES" ] || [ "$$OLD_CONFLICTING_PACKAGES" = "" ]; do \
	    OLD_CONFLICTING_PACKAGES=$$CONFLICTING_PACKAGES; \
		CONFLICTING_PACKAGES=$$( (cargo update -w --dry-run 2>&1 || true) | (grep -E '^error: failed to select a version for (the requirement )?`[^ `]+' || true) | sed -E 's|[^`]*`([^ `]+).*|\1|' | xargs); \
		if [ "$$CONFLICTING_PACKAGES" = "" ]; then \
			break; \
		fi; \
		if [ "$$OLD_CONFLICTING_PACKAGES" = "$$CONFLICTING_PACKAGES" ]; then \
			echo "Couldn't resolve Cargo conflicts" >&2; \
			exit 1; \
		fi; \
		if [ "$$CONFLICTING_PACKAGES" != "" ]; then \
			cargo update $$CONFLICTING_PACKAGES || true; \
		fi; \
	done

package:
	rm -rf $(BUILD_RELEASE); mkdir $(BUILD_RELEASE)
	(find $(CARGO_TARGET_ROOT)/release -mindepth 1 -maxdepth 1 -type f ! -name "*.*" -o -name "*.exe" -o -name "*.dll" -o -name "*.dylib" -o -name "*.so" || true) | sed -E "s|(.*)|cp -a \1 $(BUILD_RELEASE)|" | sh
	cp -a ferron-release.kdl $(BUILD_RELEASE)/ferron.kdl
	cp -a wwwroot $(BUILD_RELEASE)
	mkdir -p dist
	rm -f dist/ferron-$(FERRON_VERSION)-$(DEST_TARGET_TRIPLE).zip; cd $(BUILD_RELEASE) && zip -r ../dist/ferron-$(FERRON_VERSION)-$(DEST_TARGET_TRIPLE).zip *
	rm -rf $(BUILD_RELEASE)

package-deb:
	packaging/deb/build.sh $(DEST_TARGET_TRIPLE) $(FERRON_VERSION) $(CARGO_TARGET_ROOT)/release

package-rpm:
	packaging/rpm/build.sh $(DEST_TARGET_TRIPLE) $(FERRON_VERSION) $(CARGO_TARGET_ROOT)/release

build-with-package: build package
build-with-package-deb: build package-deb
build-with-package-rpm: build package-rpm

clean:
	rm -rf build-workspace build-release dist packaging/deb/ferron_* packaging/deb/md5sums.tmp packaging/rpm/data packaging/rpm/ferron.spec packaging/rpm/rpm
	cargo clean
	cd build-prepare && cargo clean
