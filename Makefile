help:
	@echo "This Makefile provides some useful targets to run:"
	@echo "  build-test-repo - Builds the index and starts the web server for the test repo"
	@echo "  build-mozilla-repo - Builds the index and starts the web server for the repos in mozsearch-mozilla/config1.json"
	@echo ""
	@echo "To build a local index from a try push of firefox-main:"
	@echo "  TRYPUSH_REV=7b25952b97afc2a34cc31701ffb185222727be72 make trypush # set TRYPUSH_REV to the full hg rev of your try push"

.DEFAULT_GOAL := help

.PHONY: help check-in-vagrant build-clang-plugin build-rust-tools test-rust-tools build-test-repo build-mozilla-repo baseline comparison favicon internal-check-vars internal-build-repo internal-serve-repo internal-test-repo

check-in-vagrant:
	@[ -d /vagrant ] || (echo "This command must be run inside the vagrant instance" > /dev/stderr; exit 1)

build-clang-plugin: check-in-vagrant
	$(MAKE) -C clang-plugin build_with_version_check

# This can be built outside the vagrant instance too
# We specify "--all-targets" in order to minimize rebuilding required when we invoke
# `cargo test` to validate the build.
build-rust-tools:
	cd tools && CARGO_INCREMENTAL=1 cargo install --path .
	cd scripts/web-analyze/wasm-css-analyzer && ./build.sh

test-rust-tools:
	cd tools && cargo test --release --verbose

# Building blocks for building/serving/testing repositories.
# They use different environment variable than the infrastructure scripts,
# to avoid accidentally interferring those scripts.
internal-check-vars:
	@(bash -c "[[ '$(_INDEX_ROOT)' != '' ]]" \
	|| (echo "_INDEX_ROOT is not defined" > /dev/stderr; exit 1))
	@(bash -c "[[ '$(_CONFIG_REPO)' != '' ]]" \
	|| (echo "_CONFIG_REPO is not defined" > /dev/stderr; exit 1))
	@(bash -c "[[ '$(_CONFIG_NAME)' != '' ]]" \
	|| (echo "_CONFIG_NAME is not defined" > /dev/stderr; exit 1))

internal-build-repo: internal-check-vars
	mkdir -p $(_INDEX_ROOT)
	/vagrant/infrastructure/indexer-setup.sh $(_CONFIG_REPO) $(_CONFIG_NAME) $(_INDEX_ROOT)
	/vagrant/infrastructure/indexer-run.sh $(_CONFIG_REPO) $(_INDEX_ROOT)

internal-serve-repo: _SERVER_ROOT=~
internal-serve-repo: _LOG_DIR=~
internal-serve-repo: internal-check-vars
	/vagrant/infrastructure/web-server-setup.sh $(_CONFIG_REPO) $(_CONFIG_NAME) $(_INDEX_ROOT) $(_SERVER_ROOT) $(_LOG_DIR)
	/vagrant/infrastructure/web-server-run.sh $(_CONFIG_REPO) $(_INDEX_ROOT) $(_SERVER_ROOT) $(_LOG_DIR) NO_CHANNEL NO_EMAIL WAIT

internal-test-repo: internal-check-vars
	/vagrant/infrastructure/web-server-check.sh $(_CONFIG_REPO) $(_INDEX_ROOT) "http://localhost:16995/"

build-test-repo: _INDEX_ROOT=~/index
build-test-repo: _CONFIG_REPO=/vagrant/tests
build-test-repo: _CONFIG_NAME=config.json
build-test-repo: export CHECK_WARNINGS=1
build-test-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-build-repo internal-serve-repo internal-test-repo

serve-test-repo: _INDEX_ROOT=~/index
serve-test-repo: _CONFIG_REPO=/vagrant/tests
serve-test-repo: _CONFIG_NAME=config.json
serve-test-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-serve-repo

check-test-repo: _INDEX_ROOT=~/index
check-test-repo: _CONFIG_REPO=/vagrant/tests
check-test-repo: _CONFIG_NAME=config.json
check-test-repo: internal-test-repo

# Target that:
# - Runs the check scripts in a special mode that lets the tests run without
#   failing, instead generating the revised expectations for anything that has
#   changed.
#   - We need to re-run `indexer-setup.sh` too because it rebuilds the git
#     repository and the blame repository, which need to reflect the latest
#     files, in order to make the output step see a consistent data between the
#     raw file vs the blame file.
#   - We need to re-run `indexer-run.sh` too because it embeds the disk check
#     inside `mkindex.sh`.  Arguably maybe we want to fix web-server-check.sh
#     to perhaps help run the indexer check.
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
review-test-repo: _INDEX_ROOT=~/index
review-test-repo: _CONFIG_REPO=/vagrant/tests
review-test-repo: _CONFIG_NAME=config.json
review-test-repo: export CHECK_WARNINGS=1
review-test-repo: export INSTA_FORCE_PASS=1
review-test-repo: internal-build-repo internal-serve-repo internal-test-repo
	cargo insta review --workspace-root=/vagrant/tests/tests/checks

build-searchfox-repo: _INDEX_ROOT=~/searchfox-index
build-searchfox-repo: _CONFIG_REPO=/vagrant/tests
build-searchfox-repo: _CONFIG_NAME=searchfox-config.json
build-searchfox-repo: export CHECK_WARNINGS=1
build-searchfox-repo: export MOZSEARCH_SOURCE_PATH=/vagrant
build-searchfox-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-build-repo internal-serve-repo

# Notes:
# - This also works with `export TRYPUSH_REV=full-40char-hash` for try runs
#   that have the relevant jobs scheduled on them.  In particular:
#   `./mach try fuzzy --full -q "'searchfox" -q "'bugzilla-component"`
build-mozilla-repo: _INDEX_ROOT=~/mozilla-index
build-mozilla-repo: _CONFIG_REPO=/vagrant/config
build-mozilla-repo: _CONFIG_NAME=just-mc.json
build-mozilla-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-build-repo internal-serve-repo

serve-mozilla-repo: _INDEX_ROOT=~/mozilla-index
serve-mozilla-repo: _CONFIG_REPO=/vagrant/config
serve-mozilla-repo: _CONFIG_NAME=just-mc.json
serve-mozilla-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-serve-repo

# Notes:
# - This also works with `export TRYPUSH_REV=full-40char-hash` for try runs
#   that have the relevant jobs scheduled on them.  In particular:
#   `./mach try fuzzy --full -q "'searchfox" -q "'bugzilla-component"`
build-firefox-repo: _INDEX_ROOT=~/firefox-index
build-firefox-repo: _CONFIG_REPO=/vagrant/config
build-firefox-repo: _CONFIG_NAME=just-fm.json
build-firefox-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-build-repo internal-serve-repo

serve-firefox-repo: _INDEX_ROOT=~/firefox-index
serve-firefox-repo: _CONFIG_REPO=/vagrant/config
serve-firefox-repo: _CONFIG_NAME=just-fm.json
serve-firefox-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-serve-repo

# This builds both mozsearch and mozsearch-mozilla using the trees as they exist
# on github rather than your local copies.  This differs from the
# "build-searchfox-repo" make target which uses your current tree, which can be
# useful but where anything that isn't checked-in can cause failures.  (That is,
# if you run `git status` and anything is modified, output-files can crash when
# the local checked-out status does not have the same number of lines as the
# blame repo says there should be.)
build-mozsearch-repo: _INDEX_ROOT=~/mozsearch-index
build-mozsearch-repo: _CONFIG_REPO=/vagrant/config
build-mozsearch-repo: _CONFIG_NAME=just-mozsearch.json
build-mozsearch-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-build-repo internal-serve-repo

serve-mozsearch-repo: _INDEX_ROOT=~/mozsearch-index
serve-mozsearch-repo: _CONFIG_REPO=/vagrant/config
serve-mozsearch-repo: _CONFIG_NAME=just-mozsearch.json
serve-mozsearch-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-serve-repo

build-blonk-repo: _INDEX_ROOT=~/blonk-index
build-blonk-repo: _CONFIG_REPO=/vagrant/config
build-blonk-repo: _CONFIG_NAME=config6.json
build-blonk-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-build-repo internal-serve-repo

serve-blonk-repo: _INDEX_ROOT=~/blonk-index
serve-blonk-repo: _CONFIG_REPO=/vagrant/config
serve-blonk-repo: _CONFIG_NAME=config6.json
serve-blonk-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-serve-repo

build-llvm-repo: _INDEX_ROOT=~/llvm-index
build-llvm-repo: _CONFIG_REPO=/vagrant/config
build-llvm-repo: _CONFIG_NAME=just-llvm.json
build-llvm-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-build-repo internal-serve-repo

serve-llvm-repo: _INDEX_ROOT=~/llvm-index
serve-llvm-repo: _CONFIG_REPO=/vagrant/config
serve-llvm-repo: _CONFIG_NAME=just-llvm.json
serve-llvm-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-serve-repo

build-graphviz-repo: _INDEX_ROOT=~/graphviz-index
build-graphviz-repo: _CONFIG_REPO=/vagrant/config
build-graphviz-repo: _CONFIG_NAME=just-graphviz.json
build-graphviz-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-build-repo internal-serve-repo

serve-graphviz-repo: _INDEX_ROOT=~/graphviz-index
serve-graphviz-repo: _CONFIG_REPO=/vagrant/config
serve-graphviz-repo: _CONFIG_NAME=just-graphviz.json
serve-graphviz-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-serve-repo

build-trees: _INDEX_ROOT=~/trees-index
build-trees: _CONFIG_REPO=/vagrant/tree-configs
build-trees: _CONFIG_NAME=config.json
build-trees: check-in-vagrant build-clang-plugin build-rust-tools internal-build-repo internal-serve-repo internal-test-repo

serve-trees: _INDEX_ROOT=~/trees-index
serve-trees: _CONFIG_REPO=/vagrant/tree-configs
serve-trees: _CONFIG_NAME=config.json
serve-trees: check-in-vagrant build-clang-plugin build-rust-tools internal-serve-repo

trypush: _INDEX_ROOT=~/trypush-index
trypush: _CONFIG_REPO=/vagrant/config
trypush: _CONFIG_NAME=just-mc.json
trypush: check-in-vagrant build-clang-plugin build-rust-tools internal-build-repo internal-serve-repo

nss-reblame: check-in-vagrant build-rust-tools
	mkdir -p ~/reblame
	/vagrant/infrastructure/reblame-run.sh /vagrant/config just-nss.json ~/reblame

# To test changes to indexing, run this first to generate the baseline. Then
# make your changes, and run `make comparison`. Note that we generate
# the index into ~/diffable and move it to ~/baseline so that when we
# generate the index with modifications we can also generate it into the same
# ~/diffable folder. This eliminates spurious diff results that might
# come from different absolute paths during the index generation step
baseline-prep:
	rm -rf ~/diffable ~/baseline

baseline: _INDEX_ROOT=~/diffable
baseline: _CONFIG_REPO=/vagrant/tests
baseline: _CONFIG_NAME=config.json
baseline: export MOZSEARCH_DIFFABLE=1
baseline: check-in-vagrant build-clang-plugin build-rust-tools baseline-prep internal-build-repo
	mv ~/diffable ~/baseline

comparison-prep:
	rm -rf ~/diffable ~/modified

comparison: _INDEX_ROOT=~/diffable
comparison: _CONFIG_REPO=/vagrant/tests
comparison: _CONFIG_NAME=config.json
comparison: export MOZSEARCH_DIFFABLE=1
comparison: check-in-vagrant build-clang-plugin build-rust-tools comparison-prep internal-build-repo
	mv ~/diffable ~/modified
	@echo "------------------- Below is the diff between baseline and modified. ---------------------"
	diff -u -r -x objdir ~/baseline/tests ~/modified/tests || true
	@echo "------------------- Above is the diff between baseline and modified. ---------------------"
	@echo "--- Run 'diff -u -r -x objdir ~/{baseline,modified}/tests | less' to see it in a pager ---"

build-webtest-repo: _INDEX_ROOT=~/index
build-webtest-repo: _CONFIG_REPO=/vagrant/tests
build-webtest-repo: _CONFIG_NAME=webtest-config.json
build-webtest-repo: export CHECK_WARNINGS=1
build-webtest-repo: export MOZSEARCH_SOURCE_PATH=/vagrant
build-webtest-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-build-repo internal-serve-repo

serve-webtest-repo: _INDEX_ROOT=~/index
serve-webtest-repo: _CONFIG_REPO=/vagrant/tests
serve-webtest-repo: _CONFIG_NAME=webtest-config.json
serve-webtest-repo: export MOZSEARCH_SOURCE_PATH=/vagrant
serve-webtest-repo: check-in-vagrant build-clang-plugin build-rust-tools internal-serve-repo

webtest: export MOZSEARCH_SOURCE_PATH=/vagrant
webtest: build-webtest-repo
	./scripts/webtest.sh

# Create favicon for the following 3 cases, from search.png:
#   * production.png for searchfox.org
#     the original search.png (red)
#   * testing.png for *.searchfox.org
#     vertically-mirrored, blue
#   * localhost.png for localhost
#     horizontally-mirrored, green
#
# The request to search.png is rewritten to one of them, depending on the
# hostname.  See scripts/nginx-setup.py for the rule.
#
# This operation needs to be done only when the search.png is updated.
#
# This uses `convert` command from ImageMagick.
# On the docker image, this can be installed by `sudo apt-get install imagemagick`.
favicon:
	cp ./static/icons/search.png ./static/icons/production.png
	convert -modulate 100,100,240 -flip ./static/icons/search.png ./static/icons/testing.png
	convert -modulate 50,100,180 -flop ./static/icons/search.png ./static/icons/localhost.png

rustfmt:
	git ls-files 'tools/**/*.rs' | xargs rustfmt --edition 2021
