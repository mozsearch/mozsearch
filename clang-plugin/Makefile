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

libclang-index-plugin.so: FileOperations.o StringOperations.o JSONFormatter.o MozsearchIndexer.o
	$(CXX) $(LDFLAGS) $^ -o $@ -lclangASTMatchers

check: build
	which clang
	which clang++

clean:
	$(RM) *.o *.dwo libclang-index-plugin.so

dump: build
	-clang++ -c -Xclang -ast-dump testfiles/test.cpp

analyze: build
	@if [ ! -d objdir ]; then mkdir objdir; fi
	@if [ ! -d analysis ]; then mkdir analysis; fi
	-$(RM) analysis/*
	-clang++ -c \
		-Xclang -load                       -Xclang ./libclang-index-plugin.so \
		-Xclang -add-plugin                 -Xclang mozsearch-index \
		-Xclang -plugin-arg-mozsearch-index -Xclang testfiles/ \
		-Xclang -plugin-arg-mozsearch-index -Xclang analysis/ \
		-Xclang -plugin-arg-mozsearch-index -Xclang objdir/ \
		testfiles/test.cpp
	@echo "-------------------------------------------"
	@echo "Analysis output is in the analysis/ folder"
	@echo "-------------------------------------------"

.PHONY: build clean
