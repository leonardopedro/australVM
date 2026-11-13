AUSTRAL := ../../../austral

SRC := Flux1d.aui,Flux1d.aum

.DEFAULT: all
all:
	LD_LIBRARY_PATH=../../.. $(AUSTRAL) compile $(SRC) \
		--entrypoint=Flux1d:main --use-cps-jit --output=flux1d
	./flux1d

.PHONY: all clean
clean:
	rm -f flux1d calltree.html
