CC = clang
CXX = clang++
LLVM_CONFIG ?= llvm-config
LLVM_LDFLAGS := $(shell ${LLVM_CONFIG} --ldflags)
CXXFLAGS := $(shell ${LLVM_CONFIG} --cxxflags) -fPIC -Wall -Wno-strict-aliasing \
	$(if $(DEBUG),-O0 -g)
LDFLAGS := -fPIC -g -Wl,-R -Wl,'$$ORIGIN' $(LLVM_LDFLAGS) -shared
MOZSEARCH_CLANG_PLUGIN_DIR ?= .
MOZSEARCH_CLANG_PLUGIN_OBJDIR ?= .

build: dirs $(MOZSEARCH_CLANG_PLUGIN_DIR)/libclang-index-plugin.so

dirs:
	mkdir -p from-clangd
	mkdir -p $(MOZSEARCH_CLANG_PLUGIN_OBJDIR)
	mkdir -p $(MOZSEARCH_CLANG_PLUGIN_OBJDIR)/from-clangd
	mkdir -p $(MOZSEARCH_CLANG_PLUGIN_DIR)

build_with_version_check: clang-version-guard build

clang-version-guard:
	@$(CXX) --version > clang-version.cache.tmp
	@if [ -f clang-version.cache ]; then \
		if ! diff -q clang-version.cache clang-version.cache.tmp; then \
			echo "Clang version mismatch is detected."; \
			echo "Rebuilding from scratch..."; \
			mv clang-version.cache.tmp clang-version.cache; \
			make clean; \
		else \
			rm clang-version.cache.tmp; \
		fi; \
	else \
		echo "Previous clang version is not found."; \
		echo "Rebuilding from scratch..."; \
		mv clang-version.cache.tmp clang-version.cache; \
		make clean; \
	fi

$(MOZSEARCH_CLANG_PLUGIN_OBJDIR)/%.o: %.cpp
	$(CXX) $(CXXFLAGS) -c $^ -o $@

$(MOZSEARCH_CLANG_PLUGIN_OBJDIR)/from-clangd/%.o: from-clangd/%.cpp
	$(CXX) $(CXXFLAGS) -c $^ -o $@

$(MOZSEARCH_CLANG_PLUGIN_DIR)/libclang-index-plugin.so: $(MOZSEARCH_CLANG_PLUGIN_OBJDIR)/FileOperations.o $(MOZSEARCH_CLANG_PLUGIN_OBJDIR)/StringOperations.o $(MOZSEARCH_CLANG_PLUGIN_OBJDIR)/BindingOperations.o $(MOZSEARCH_CLANG_PLUGIN_OBJDIR)/MozsearchIndexer.o $(MOZSEARCH_CLANG_PLUGIN_OBJDIR)/from-clangd/HeuristicResolver.o
	$(CXX) $(LDFLAGS) $^ -o $@ -lclangASTMatchers

check: build
	which clang
	which clang++

clean: dirs
clean:
	$(RM) $(MOZSEARCH_CLANG_PLUGIN_OBJDIR)/*.o $(MOZSEARCH_CLANG_PLUGIN_OBJDIR)/from-clangd/*.o *.dwo $(MOZSEARCH_CLANG_PLUGIN_DIR)/libclang-index-plugin.so

dump: build
	-clang++ -c -Xclang -ast-dump testfiles/test.cpp

# When NFS is used on linux under libvirt, the file-locking is a real problem,
# so let's not use an immediate subdirectory of this dir.
ANALYSIS_DIR := /tmp/searchfox-analysis
analyze: build
	@if [ ! -d objdir ]; then mkdir objdir; fi
	-mkdir -p $(ANALYSIS_DIR)
	-$(RM) $(ANALYSIS_DIR)/*
	$(CXX) -c \
		-Xclang -load                       -Xclang $(MOZSEARCH_CLANG_PLUGIN_DIR)/libclang-index-plugin.so \
		-Xclang -add-plugin                 -Xclang mozsearch-index \
		-Xclang -plugin-arg-mozsearch-index -Xclang testfiles/ \
		-Xclang -plugin-arg-mozsearch-index -Xclang $(ANALYSIS_DIR) \
		-Xclang -plugin-arg-mozsearch-index -Xclang objdir/ \
		testfiles/test.cpp
	@echo "-------------------------------------------"
	@echo "Analysis output is in the ${ANALYSIS_DIR}/ folder"
	@echo "-------------------------------------------"
	@echo "maybe type:"
	@echo "vim ${ANALYSIS_DIR}/test.cpp"

.PHONY: build clean clang-version-guard dirs
