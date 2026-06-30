VERSION_XCCONFIG := Config/Version.xcconfig
VERSION := $(shell sed -n 's/^MARKETING_VERSION = \([0-9][0-9.]*\).*/\1/p' $(VERSION_XCCONFIG))
BUILD_NUMBER := $(shell git rev-list --count HEAD 2>/dev/null || echo 0)

.PHONY: test build version clean

test:
	swift test

build:
	swift build

## Print the resolved marketing version and (monotonic) build number.
version:
	@echo "version=$(VERSION) build=$(BUILD_NUMBER)"

clean:
	swift package clean
	rm -rf .build
