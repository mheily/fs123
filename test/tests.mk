# This is a Makefile, but it's not meant to be run "standalone".  It
# can only be run after extensive setup, as is done in ./runtests.
#
# We're using make because it's an effective way to encapsulate
# parallelism.  It's typically run as:
#   make -f tests.mk -B -j partests
#   make -f tests.mk -B seqtests
# so there's no need to jump through hoops identifying prerequisites.

# Also note that the seqtests *MUST NOT* be run with -j.  They fiddle
# with the client and server configuration in ways that don't play
# well with others.

testdir/ := $(dir $(word $(words $(MAKEFILE_LIST)),$(MAKEFILE_LIST)))
.PHONY: partests seqtests

partests: $(addsuffix .out, $(notdir $(wildcard $(testdir/)t-*[a-zA-Z0-9])))
seqtests: $(addsuffix .out, $(notdir $(wildcard $(testdir/)tseq-*[a-zA-Z0-9])))

%.out :
	timeout 2m sh -c "$(testdir/)$* > $@ 2>&1"



