help:
	@echo "This Makefile provides some useful targets to run:"
	@echo "  build-test-repo - Builds the index and starts the web server for the test repo"
	@echo "  build-mozilla-repo - Builds the index and starts the web server for the repos in mozsearch-mozilla/config1.json"
	@echo ""
	@echo "To build a local index from a try push of mozilla-central:"
	@echo "  TRYPUSH_REV=7b25952b97afc2a34cc31701ffb185222727be72 make trypush # set TRYPUSH_REV to the full hg rev of your try push"

.DEFAULT_GOAL := help

.PHONY: help check-in-vagrant build-clang-plugin build-rust-tools test-rust-tools build-test-repo build-mozilla-repo baseline comparison

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
	/vagrant/infrastructure/indexer-setup.sh /vagrant/tests config.json ~/index
	/vagrant/infrastructure/indexer-run.sh /vagrant/tests ~/index
	/vagrant/infrastructure/web-server-setup.sh /vagrant/tests config.json ~/index ~
	/vagrant/infrastructure/web-server-run.sh /vagrant/tests ~/index ~
	/vagrant/infrastructure/web-server-check.sh /vagrant/tests ~/index "http://localhost/"

check-test-repo:
	/vagrant/infrastructure/web-server-check.sh /vagrant/tests ~/index "http://localhost/"

# Target that:
# - Runs the check script in a special mode that lets the tests run without
#   failing, instead generating the revised expectations for anything that has
#   changed.
# - Runs the `cargo insta review` command which has a cool interactive UI that
#   shows any differences.
#
# You would likely want to run this if:
# - You ran `make build-test-repo` and got errors and you were like, "oh, yeah,
#   stuff might have changed and I should look at it and maybe approve those
#   changes."
# - You know you already have changed stuff and need to review those changes.
#
# Depends on `cargo install cargo-insta`.
review-test-repo:
	INSTA_FORCE_PASS=1 /vagrant/infrastructure/web-server-check.sh /vagrant/tests ~/index "http://localhost/"
	cargo insta review --workspace-root=/vagrant/tests/

build-searchfox-repo: check-in-vagrant build-clang-plugin build-rust-tools
	mkdir -p ~/searchfox-index
	/vagrant/infrastructure/indexer-setup.sh /vagrant/tests searchfox-config.json ~/searchfox-index
	/vagrant/infrastructure/indexer-run.sh /vagrant/tests ~/searchfox-index
	/vagrant/infrastructure/web-server-setup.sh /vagrant/tests searchfox-config.json ~/searchfox-index ~
	/vagrant/infrastructure/web-server-run.sh /vagrant/tests ~/searchfox-index ~

build-mozilla-repo: check-in-vagrant build-clang-plugin build-rust-tools
	[ -d ~/mozilla-config ] || git clone https://github.com/mozsearch/mozsearch-mozilla ~/mozilla-config
	mkdir -p ~/mozilla-index
	/vagrant/infrastructure/indexer-setup.sh ~/mozilla-config config1.json ~/mozilla-index
	/vagrant/infrastructure/indexer-run.sh ~/mozilla-config ~/mozilla-index
	/vagrant/infrastructure/web-server-setup.sh ~/mozilla-config config1.json ~/mozilla-index ~
	/vagrant/infrastructure/web-server-run.sh ~/mozilla-config ~/mozilla-index ~

# This is similar to build-mozilla-repo, except it strips out the non-mozilla-central trees
# from config.json and puts the stripped version into trypush.json.
trypush: check-in-vagrant build-clang-plugin build-rust-tools
	[ -d ~/mozilla-config ] || git clone https://github.com/mozsearch/mozsearch-mozilla ~/mozilla-config
	jq '{mozsearch_path, default_tree, trees: {"mozilla-central": .trees["mozilla-central"]}}' ~/mozilla-config/config1.json > ~/mozilla-config/trypush.json
	mkdir -p ~/trypush-index
	/vagrant/infrastructure/indexer-setup.sh ~/mozilla-config trypush.json ~/trypush-index
	/vagrant/infrastructure/indexer-run.sh ~/mozilla-config ~/trypush-index
	/vagrant/infrastructure/web-server-setup.sh ~/mozilla-config trypush.json ~/trypush-index ~
	/vagrant/infrastructure/web-server-run.sh ~/mozilla-config ~/trypush-index ~

nss-reblame: check-in-vagrant build-rust-tools
	[ -d ~/mozilla-config ] || git clone https://github.com/mozsearch/mozsearch-mozilla ~/mozilla-config
	jq '{mozsearch_path, default_tree, trees: {"nss": .trees["nss"]}}' ~/mozilla-config/config1.json > ~/mozilla-config/nss.json
	mkdir -p ~/reblame
	/vagrant/infrastructure/reblame-run.sh ~/mozilla-config nss.json ~/reblame

# To test changes to indexing, run this first to generate the baseline. Then
# make your changes, and run `make comparison`. Note that we generate
# the index into ~/diffable and move it to ~/baseline so that when we
# generate the index with modifications we can also generate it into the same
# ~/diffable folder. This eliminates spurious diff results that might
# come from different absolute paths during the index generation step
baseline: check-in-vagrant build-clang-plugin build-rust-tools
	rm -rf ~/diffable ~/baseline
	mkdir -p ~/diffable
	/vagrant/infrastructure/indexer-setup.sh /vagrant/tests config.json ~/diffable
	MOZSEARCH_DIFFABLE=1 /vagrant/infrastructure/indexer-run.sh /vagrant/tests ~/diffable
	mv ~/diffable ~/baseline

comparison: check-in-vagrant build-clang-plugin build-rust-tools
	rm -rf ~/diffable ~/modified
	mkdir -p ~/diffable
	/vagrant/infrastructure/indexer-setup.sh /vagrant/tests config.json ~/diffable
	MOZSEARCH_DIFFABLE=1 /vagrant/infrastructure/indexer-run.sh /vagrant/tests ~/diffable
	mv ~/diffable ~/modified
	@echo "------------------- Below is the diff between baseline and modified. ---------------------"
	diff -u -r -x objdir ~/baseline/tests ~/modified/tests || true
	@echo "------------------- Above is the diff between baseline and modified. ---------------------"
	@echo "--- Run 'diff -u -r -x objdir ~/{baseline,modified}/tests | less' to see it in a pager ---"
