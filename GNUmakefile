# This Makefile assumes that fs123's prerequisites are met.
# See README.md for the prerequisites.
#
# Tell make where to find binaries and libraries installed in
# non-standard locations by setting environment variables: CC, CXX,
# TARGET_ARCH, CPPFLAGS, CXXFLAGS, CFLAGS, LDFLAGS, LDLIBS.
#
# Targets and (many!) intermediate files will be created in the
# directory from which make is run.  Out-of-tree builds are supported.
# So to avoid cluttering the source tree, 
#   mkdir build
#   cd build
#   make -f ../GNUmakefile
#
# See http://make.mad-scientist.net/papers/multi-architecture-builds/
# for a possible alternative...
.SECONDARY:
ifeq ($(origin created_by_configure), undefined)
# Assume we're running in the source tree.
top/ := $(dir $(word $(words $(MAKEFILE_LIST)),$(MAKEFILE_LIST)))
else
top/ := @@srcdir@@/
endif
abstop/ := $(realpath $(top/))/
VPATH=$(top/)lib:$(top/)exe/fs123p7:$(top/)exe/examples:$(top/)exe/testserver
# Link with $(CXX), not $(CC)!
LINK.o = $(CXX) $(LDFLAGS) $(TARGET_ARCH)

# First, let's define the 'all' target so a build with no arguements
# does something sensible:
binaries=fs123p7

unit_tests=ut_diskcache
unit_tests += ut_seektelldir
unit_tests += ut_content_codec
unit_tests += ut_cc_rules
unit_tests += ut_inomap

# other_exe
other_exe = ex1server testserver

libs=libfs123.a

EXE = $(binaries) $(unit_tests) $(other_exe)

.PHONY: all
all: $(EXE)

.PHONY: check
check: $(EXE)
	$(top/)test/runtests

.PHONY: core123-check
core123-check:
	-mkdir core123-build
	$(MAKE) -C core123-build -f $(abstop/)core123/GNUmakefile check

OPT?=-O3 # if not explicitly set
DASHG?=-ggdb
CPPFLAGS := -iquote $(top/)include $(CPPFLAGS)
CPPFLAGS := -I $(top/)core123/include $(CPPFLAGS)
CXXFLAGS += -DFUSE_USE_VERSION=26
CXXFLAGS += -std=c++17 -Wall
CXXFLAGS += -Wshadow
CXXFLAGS += -Werror
CXXFLAGS += -Wextra
CXXFLAGS += -D_FILE_OFFSET_BITS=64
CXXFLAGS += $(OPT)
CXXFLAGS += $(DASHG)

CFLAGS += -std=c99
CFLAGS += $(OPT)
CFLAGS += $(DASHG)
LDFLAGS += -pthread

ifeq ($(origin GIT_DESCRIPTION), undefined)
  GIT_DESCRIPTION:=$(shell cd $(top/); git describe --always --dirty 2>/dev/null || echo not-git)
endif
CXXFLAGS += -DGIT_DESCRIPTION=\"$(GIT_DESCRIPTION)\"

serverlibs=-levent -levent_pthreads -lsodium

# < libfs123 >
libfs123_cppsrcs:=content_codec.cpp secret_manager.cpp sharedkeydir.cpp fs123server.cpp
CPPSRCS += $(libfs123_cppsrcs)
libfs123_objs:=$(libfs123_cppsrcs:%.cpp=%.o)
libfs123.a : $(libfs123_objs)
	$(AR) $(ARFLAGS) $@ $?

# ut_content_codec needs libsodium
ut_content_codec : LDLIBS += -lsodium

# < /libfs123 >

# <fs123p7>
fs123p7_cppsrcs:=fs123p7.cpp app_mount.cpp app_setxattr.cpp app_ctl.cpp fuseful.cpp backend123.cpp backend123_http.cpp diskcache.cpp special_ino.cpp inomap.cpp openfilemap.cpp distrib_cache_backend.cpp
fs123p7_cppsrcs += app_exportd.cpp exportd_handler.cpp exportd_cc_rules.cpp
CPPSRCS += $(fs123p7_cppsrcs)
fs123p7_objs :=$(fs123p7_cppsrcs:%.cpp=%.o)

fs123p7_csrcs:=opensslthreadlock.c
CSRCS += $(fs123p7_csrcs)
fs123p7_objs +=$(fs123p7_csrcs:%.c=%.o)

ifndef NO_OPENSSL
fs123p7 : LDLIBS += -lcrypto
endif
FUSELIB?=fuse # if not  explicitly set
fs123p7 : LDLIBS += -l$(FUSELIB)
fs123p7 : LDLIBS += $(serverlibs)
# curllibs: either 'curl-config --libs' or 'curl-config --static-libs'
curllibs=$(shell curl-config $(if $(filter -static,$(LDFLAGS)), --static-libs, --libs))
# put curllibs at the front so if curl-config is incomplete,
# we can satisfy its dependencies with an explicit LDLIBS.
fs123p7 : LDLIBS := $(curllibs) $(LDLIBS)
fs123p7 : $(fs123p7_objs)

# link ut_diskcache links with some client-side .o files
ut_diskcache : diskcache.o backend123.o 
ut_inomap : inomap.o
ut_cc_rules : exportd_cc_rules.o

backend123_http.o : CPPFLAGS += $(shell curl-config --cflags)
#</fs123p7>


# <ex1server>
ex1_cppsrcs := ex1server.cpp
CPPSRCS += $(ex1_cppsrcs)
ex1_objs := $(ex1_cppsrcs:%.cpp=%.o)
ex1server: LDLIBS += $(serverlibs)
ex1server: $(ex1_objs)
	$(LINK.o) $^ $(LOADLIBES) $(LDLIBS) -o $@
# </ex1server>
# <testserver>
testserver_cppsrcs := testserver.cpp
CPPSRCS += $(testserver_cppsrcs)
testserver_objs := $(testserver_cppsrcs:%.cpp=%.o)
testserver: LDLIBS += $(serverlibs)
testserver: $(testserver_objs)
	$(LINK.o) $^ $(LOADLIBES) $(LDLIBS) -o $@
# </testserver>

# Why doesn't this work if it's higher up?
$(EXE): libfs123.a

.PHONY: clean
clean:
	rm -f $(EXE) *.o *.gcno *.gcda *.gcov *.a *.so
	[ ! -d core123-build ] || rm -rf core123-build
	[ ! -d "$(DEPDIR)" ] || rm -rf $(DEPDIR)

export prefix?=/usr
export bindir?=$(prefix)/bin
export sbindir?=$(prefix)/sbin
export libdir?=$(prefix)/lib
export includedir?=$(prefix)/include

.PHONY: install
install : $(binaries) $(libs)
	mkdir -p $(DESTDIR)$(includedir) $(DESTDIR)$(libdir) $(DESTDIR)$(bindir) $(DESTDIR)$(sbindir)
	cp -a $(binaries) $(DESTDIR)$(bindir)
	ln -s -f $(bindir)/fs123p7 $(DESTDIR)$(sbindir)/mount.fs123p7
	cp -a $(libs) $(DESTDIR)$(libdir)
	cp -a $(top/)include/fs123 $(DESTDIR)$(includedir)
	$(MAKE) -f $(top/)core123/GNUmakefile install

# <autodepends from http://make.mad-scientist.net/papers/advanced-auto-dependency-generation>
# Modified to work with CSRCS and CPPSRCS instead of just SRCS...
# Also modified to add && touch $@ to the recipe because on MacOS
# the .d file gets written after the .o file.  What could possibly go wrong?
DEPDIR := .deps
DEPFLAGS = -MT $@ -MMD -MP -MF $(DEPDIR)/$*.d

COMPILE.c = $(CC) $(DEPFLAGS) $(CFLAGS) $(CPPFLAGS) $(TARGET_ARCH) -c

%.o : %.c
%.o : %.c $(DEPDIR)/%.d | $(DEPDIR)
	$(COMPILE.c) $(OUTPUT_OPTION) $< && touch $@

COMPILE.cpp = $(CXX) $(DEPFLAGS) $(CXXFLAGS) $(CPPFLAGS) $(TARGET_ARCH) -c
%.o : %.cpp
%.o : %.cpp $(DEPDIR)/%.d | $(DEPDIR)
	$(COMPILE.cpp) $(OUTPUT_OPTION) $< && touch $@

$(DEPDIR): ; @mkdir -p $@

DEPFILES := $(CSRCS:%.c=$(DEPDIR)/%.d) $(CPPSRCS:%.cpp=$(DEPDIR)/%.d)
$(DEPFILES):
include $(wildcard $(DEPFILES))
# </autodepends>
