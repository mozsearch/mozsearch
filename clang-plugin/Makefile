LLVM_CONFIG ?= llvm-config
LLVM_LDFLAGS := $(shell ${LLVM_CONFIG} --ldflags)
CXXFLAGS := $(shell ${LLVM_CONFIG} --cxxflags) -Wall -Wno-strict-aliasing \
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

.PHONY: build clean
