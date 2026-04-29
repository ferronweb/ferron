all: build
build: prepare bundle

prepare:
	rm -rf staging && cp -r src staging && cp ../configs/ferron.pkgunix.conf staging/ferron.conf

bundle:
	if [ ! -d ../dist ]; then mkdir ../dist; fi && \
	echo '#!/bin/sh' > ../dist/install.sh && \
	echo 'FERRON_INSTALLER_EXTRACT_DIR=$$(mktemp -d)' >> ../dist/install.sh && \
    for script in $$(find staging -type f); do \
		SCRIPT_BASE=$$(echo $$script | sed 's/^staging\///') && \
		SCRIPT_DIR=$$(dirname $$SCRIPT_BASE) && \
		if [ "$$SCRIPT_DIR" != "." ]; then \
			echo 'mkdir -p "$$FERRON_INSTALLER_EXTRACT_DIR/'$$SCRIPT_DIR'"' >> ../dist/install.sh; \
		fi && \
		echo 'sed s/^X//g > "$$FERRON_INSTALLER_EXTRACT_DIR/'$$SCRIPT_BASE'" << '"'"'EOF'"'" >> ../dist/install.sh; \
		cat $$script | sed 's/^/X/g' >> ../dist/install.sh; \
		echo 'EOF' >> ../dist/install.sh; \
	done && \
	echo '. $$FERRON_INSTALLER_EXTRACT_DIR/main.sh' >> ../dist/install.sh && \
	chmod +x ../dist/install.sh
