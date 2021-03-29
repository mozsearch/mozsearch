CC = clang
CXX = clang++
LLVM_CONFIG ?= llvm-config
LLVM_LDFLAGS := $(shell ${LLVM_CONFIG} --ldflags)
CXXFLAGS := $(shell ${LLVM_CONFIG} --cxxflags) -fPIC -Wall -Wno-strict-aliasing \
	$(if $(DEBUG),-O0 -g)
LDFLAGS := -fPIC -g -Wl,-R -Wl,'$$ORIGIN' $(LLVM_LDFLAGS) -shared

build: libclang-index-plugin.so

%.o: %.cpp
	$(CXX) $(CXXFLAGS) -c $^ -o $@

libclang-index-plugin.so: FileOperations.o StringOperations.o MozsearchIndexer.o
	$(CXX) $(LDFLAGS) $^ -o $@ -lclangASTMatchers

check: build
	which clang
	which clang++

clean:
	$(RM) *.o *.dwo libclang-index-plugin.so

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
		-Xclang -load                       -Xclang ./libclang-index-plugin.so \
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

.PHONY: build clean
