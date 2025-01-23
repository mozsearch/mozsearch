help:
	@echo "This Makefile provides some useful targets to run:"
	@echo "  build-test-repo - Builds the index and starts the web server for the test repo"
	@echo "  build-mozilla-repo - Builds the index and starts the web server for the repos in mozsearch-mozilla/config1.json"
	@echo ""
	@echo "To build a local index from a try push of mozilla-central:"
	@echo "  TRYPUSH_REV=7b25952b97afc2a34cc31701ffb185222727be72 make trypush # set TRYPUSH_REV to the full hg rev of your try push"

.DEFAULT_GOAL := help

.PHONY: help build-clang-plugin build-rust-tools test-rust-tools build-test-repo build-mozilla-repo baseline comparison

NIXFLAGS := -L --accept-flake-config --override-input self-with-dotgit path:$(shell pwd)
MOZSEARCH_MOZILLA := git+https://github.com/nicolas-guichard/mozsearch-mozilla?ref=nixified

build-clang-plugin:
	nix build $(NIXFLAGS) '.?submodules=1#mozsearch-clang-plugin' --no-link

# This can be built outside the vagrant instance too
# We specify "--all-targets" in order to minimize rebuilding required when we invoke
# `cargo test` to validate the build.
build-rust-tools:
	nix build $(NIXFLAGS) '.?submodules=1#mozsearch-rust-tools' '.?submodules=1#mozsearch-wasm-css-analyzer' --no-link

test-rust-tools: build-rust-tools

build-test-repo:
	$(eval INDEX := $(shell nix build $(NIXFLAGS) '.?submodules=1#tests.unchecked' --no-link --print-out-paths))
	nix run $(NIXFLAGS) '.?submodules=1#serve-index' -- $(INDEX)/index srv

serve-test-repo: build-test-repo

check-test-repo: serve-test-repo
	SEARCHFOX_SERVER=http://localhost:16995/ SEARCHFOX_TREE=tests INSTA_WORKSPACE_ROOT=$(shell pwd)/tests/tests/checks nix run $(NIXFLAGS) '.?submodules=1#test-index' -- $(shell pwd)/tests/tests/checks

# Target that:
# - Runs the check scripts in a special mode that lets the tests run without
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
review-test-repo: serve-test-repo
	INSTA_FORCE_PASS=1 SEARCHFOX_SERVER=http://localhost:16995/ SEARCHFOX_TREE=tests INSTA_WORKSPACE_ROOT=$(shell pwd)/tests/tests/checks nix run $(NIXFLAGS) '.?submodules=1#test-index' -- $(shell pwd)/tests/tests/checks
	nix run $(NIXFLAGS) '.?submodules=1#review-snapshots'

build-searchfox-repo:
	nix build $(NIXFLAGS) '.?submodules=1#searchfox' --no-link --print-out-paths

# Notes:
# - If you want to use a modified version of mozsearch-mozilla, such as one
#   checked out under "config" in the check-out repo, you can create a symlink
#   in the VM's home directory via `pushd ~; ln -s /vagrant/config mozilla-config`.
# - This also works with `export TRYPUSH_REV=full-40char-hash` for try runs
#   that have the relevant jobs scheduled on them.  In particular:
#   `./mach try fuzzy --full -q "'searchfox" -q "'bugzilla-component"`
build-mozilla-repo:
	nix run $(NIXFLAGS) '$(MOZSEARCH_MOZILLA)#index-just-mc' --override-input mozsearch '.?submodules=1' -- ~/mozilla-index
	$(serve-mozilla-repo)

serve-mozilla-repo:
	nix run $(NIXFLAGS) '.?submodules=1#serve-index' -- ~/mozilla-index ~/mozilla-srv

# This builds both mozsearch and mozsearch-mozilla using the trees as they exist
# on github rather than your local copies.  This differs from the
# "build-searchfox-repo" make target which uses your current tree, which can be
# useful but where anything that isn't checked-in can cause failures.  (That is,
# if you run `git status` and anything is modified, output-files can crash when
# the local checked-out status does not have the same number of lines as the
# blame repo says there should be.)
#
# Notes:
# - If you want to use a modified version of mozsearch-mozilla, such as one
#   checked out under "config" in the check-out repo, you can create a symlink
#   in the VM's home directory via `pushd ~; ln -s /vagrant/config mozsearch-config`.
build-mozsearch-repo:
	nix run $(NIXFLAGS) 'MOZSEARCH_MOZILLA?ref=nixified#index-just-mozsearch' --override-input mozsearch '.?submodules=1' -- ~/mozsearch-index
	$(serve-mozsearch-repo)

serve-mozsearch-repo:
	nix run $(NIXFLAGS) '.?submodules=1#serve-index' -- ~/mozsearch-index ~/mozsearch-srv

# Notes:
# - If you want to use a modified version of mozsearch-mozilla, such as one
#   checked out under "config" in the check-out repo, you can create a symlink
#   in the VM's home directory via `pushd ~; ln -s /vagrant/config llvm-config`.
build-llvm-repo:
	nix run $(NIXFLAGS) 'MOZSEARCH_MOZILLA?ref=nixified#index-just-llvm' --override-input mozsearch '.?submodules=1' -- ~/mozsearch-index
	$(serve-llvm-repo)

serve-llvm-repo:
	nix run $(NIXFLAGS) '.?submodules=1#serve-index' -- ~/llvm-index ~/llvm-srv

# Notes:
# - If you want to use a modified version of mozsearch-mozilla, such as one
#   checked out under "config" in the check-out repo, you can create a symlink
#   in the VM's home directory via `pushd ~; ln -s /vagrant/config graphviz-config`.
build-graphviz-repo:
	nix run $(NIXFLAGS) 'MOZSEARCH_MOZILLA?ref=nixified#index-just-graphviz' --override-input mozsearch '.?submodules=1#' -- ~/graphviz-index
	$(serve-graphviz-repo)

serve-graphviz-repo:
	nix run $(NIXFLAGS) '.?submodules=1#serve-index' -- ~/graphviz-index ~/graphviz-srv

build-trees:
	nix run $(NIXFLAGS) '.?submodules=1#build-index' -- $(shell pwd)/tree-configs config.json ~/trees-index
	$(serve-trees)

serve-trees:
	nix run $(NIXFLAGS) '.?submodules=1#serve-index' -- ~/trees-index ~/trees-srv

# This is similar to build-mozilla-repo, except it strips out the non-mozilla-central trees
# from config.json and puts the stripped version into trypush.json.
trypush: check-in-vagrant build-clang-plugin build-rust-tools
	[ -d ~/mozilla-config ] || git clone https://github.com/mozsearch/mozsearch-mozilla ~/mozilla-config
	jq '{mozsearch_path, config_repo, default_tree, trees: {"mozilla-central": .trees["mozilla-central"]}}' ~/mozilla-config/config1.json > ~/mozilla-config/trypush.json
	mkdir -p ~/trypush-index
	/vagrant/infrastructure/indexer-setup.sh ~/mozilla-config trypush.json ~/trypush-index
	/vagrant/infrastructure/indexer-run.sh ~/mozilla-config ~/trypush-index
	/vagrant/infrastructure/web-server-setup.sh ~/mozilla-config trypush.json ~/trypush-index ~
	/vagrant/infrastructure/web-server-run.sh ~/mozilla-config ~/trypush-index ~

nss-reblame: check-in-vagrant build-rust-tools
	[ -d ~/mozilla-config ] || git clone https://github.com/mozsearch/mozsearch-mozilla ~/mozilla-config
	jq '{mozsearch_path, config_repo, default_tree, trees: {"nss": .trees["nss"]}}' ~/mozilla-config/config1.json > ~/mozilla-config/nss.json
	mkdir -p ~/reblame
	/vagrant/infrastructure/reblame-run.sh ~/mozilla-config nss.json ~/reblame

# To test changes to indexing, run this first to generate the baseline. Then
# make your changes, and run `make comparison`. Note that we generate
# the index into ~/diffable and move it to ~/baseline so that when we
# generate the index with modifications we can also generate it into the same
# ~/diffable folder. This eliminates spurious diff results that might
# come from different absolute paths during the index generation step
baseline:
	unlink ~/baseline
	$(eval INDEX := $(shell nix build $(NIXFLAGS) '.?submodules=1#tests.diffable' --no-link --print-out-paths))
	ln -s $(INDEX)/index ~/baseline

comparison:
	unlink ~/modified
	$(eval INDEX := $(shell nix build $(NIXFLAGS) '.?submodules=1#tests.diffable' --no-link --print-out-paths))
	ln -s $(INDEX)/index ~/modified
	@echo "------------------- Below is the diff between baseline and modified. ---------------------"
	diff -u -r -x objdir ~/baseline/tests ~/modified/tests || true
	@echo "------------------- Above is the diff between baseline and modified. ---------------------"
	@echo "--- Run 'diff -u -r -x objdir ~/{baseline,modified}/tests | less' to see it in a pager ---"

build-webtest-repo:
	nix build $(NIXFLAGS) '.?submodules=1#webtests.unchecked' --no-link --print-out-paths

webtest:
	nix build $(NIXFLAGS) '.?submodules=1#webtests' --no-link --print-out-paths
