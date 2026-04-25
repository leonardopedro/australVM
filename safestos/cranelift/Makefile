test: test_bridge
	./test_bridge

test_bridge: test_bridge.c
	gcc -o test_bridge test_bridge.c -L. -laustral_cranelift_bridge -Wl,-rpath,./target/release

test_bridge.c:
	# Created during session

.PHONY: test
