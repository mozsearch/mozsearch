help:
	@echo "This Makefile provides some useful targets to run:"
	@echo "  build-test-repo - Builds the index and starts the web server for the test repo"
	@echo "  build-mozilla-repo - Builds the index and starts the web server for the mozsearch-mozilla repo"

.DEFAULT_GOAL := help

.PHONY: help check-in-vagrant build-clang-plugin build-rust-tools test-rust-tools build-test-repo build-mozilla-repo

check-in-vagrant:
	@[ -d /vagrant ] || (echo "This command must be run inside the vagrant instance" > /dev/stderr; exit 1)

build-clang-plugin: check-in-vagrant
	$(MAKE) -C clang-plugin

# This can be built outside the vagrant instance too
build-rust-tools:
	cd tools && rustup run nightly cargo build --release

test-rust-tools:
	cd tools && rustup run nightly cargo test --release --verbose

build-test-repo: check-in-vagrant build-clang-plugin build-rust-tools
	mkdir -p ~/index
	/vagrant/infrastructure/indexer-setup.sh /vagrant/tests ~/index
	/vagrant/infrastructure/indexer-run.sh /vagrant/tests ~/index
	/vagrant/infrastructure/web-server-setup.sh /vagrant/tests ~/index ~
	/vagrant/infrastructure/web-server-run.sh /vagrant/tests ~/index ~

build-mozilla-repo: check-in-vagrant build-clang-plugin build-rust-tools
	[ -d ~/mozilla-config ] || git clone https://github.com/mozsearch/mozsearch-mozilla ~/mozilla-config
	mkdir -p ~/mozilla-index
	/vagrant/infrastructure/indexer-setup.sh ~/mozilla-config ~/mozilla-index
	/vagrant/infrastructure/indexer-run.sh ~/mozilla-config ~/mozilla-index
	/vagrant/infrastructure/web-server-setup.sh ~/mozilla-config ~/mozilla-index ~
	/vagrant/infrastructure/web-server-run.sh ~/mozilla-config ~/mozilla-index ~
