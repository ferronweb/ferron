FERRON_VERSION_CARGO = $(shell cat ferron/Cargo.toml | grep -E '^version' | sed -E 's|.*"([0-9a-zA-Z.+-]+)"$$|\1|g')
FERRON_VERSION_GIT = $(shell git tag --sort=-committerdate | head -n 1 | sed s/[^0-9a-zA-Z.+-]//g)
ifdef FERRON_VERSION_CARGO
	FERRON_VERSION = $(FERRON_VERSION_CARGO)
else
	FERRON_VERSION = $(FERRON_VERSION_GIT)
endif
HOST_TARGET_TRIPLE = $(shell rustc -vV | sed -n 's|host: ||p')

ifdef TARGET
	CARGO_FINAL_EXTRA_ARGS = --target $(TARGET)
	CARGO_TARGET_ROOT = target/$(TARGET)
	DEST_TARGET_TRIPLE = $(TARGET)
	BUILD_RELEASE = build-release-$(TARGET)
else
	CARGO_FINAL_EXTRA_ARGS =
	CARGO_TARGET_ROOT = target
	DEST_TARGET_TRIPLE = $(HOST_TARGET_TRIPLE)
	BUILD_RELEASE = build-release
endif

ifndef CARGO_FINAL
	CARGO_FINAL = cargo
endif

.PHONY: build

run: build
	ifneq ($(TARGET),$(HOST_TARGET_TRIPLE))
	    $(error "Cannot run cross-compiled binaries.")
	endif
	target/release/ferron

run-dev: build-dev
	ifneq ($(TARGET),$(HOST_TARGET_TRIPLE))
	    $(error "Cannot run cross-compiled binaries.")
	endif
	target/debug/ferron

build: prepare-build
	cd build-workspace && $(CARGO_FINAL) build --target-dir ../target -r $(CARGO_FINAL_EXTRA_ARGS)

build-dev: prepare-build
	cd build-workspace && $(CARGO_FINAL) build --target-dir ../target $(CARGO_FINAL_EXTRA_ARGS)

prepare-build:
	cargo run --manifest-path build-prepare/Cargo.toml

package:
	rm -rf $(BUILD_RELEASE); mkdir $(BUILD_RELEASE)
	find $(CARGO_TARGET_ROOT)/release -mindepth 1 -maxdepth 1 -type f ! -name "*.*" -o -name "*.exe" -o -name "*.dll" -o -name "*.dylib" -o -name "*.so" | sed -E "s|(.*)|cp -a \1 $(BUILD_RELEASE)|" | sh
	cp -a ferron-release.kdl $(BUILD_RELEASE)/ferron.kdl
	cp -a wwwroot $(BUILD_RELEASE)
	zip -r dist/ferron-$(FERRON_VERSION)-$(DEST_TARGET_TRIPLE).zip $(BUILD_RELEASE)/*
	rm -rf $(BUILD_RELEASE)

build-with-package: build package

clean:
	rm -rf build-workspace build-release dist
	cargo clean
