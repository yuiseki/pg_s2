PG_MAJOR ?= 17
PG_CONFIG ?= /usr/lib/postgresql/$(PG_MAJOR)/bin/pg_config
BUILD_DIR ?= build
PGRX_HOME ?= /root/.pgrx
EXT_NAME := pg_s2
DOCKER_COMPOSE ?= docker compose
DEV_SERVICE ?= dev

.PHONY: build test package compose-build build-in-container test-in-container package-in-container init-in-container clean

compose-build:
	$(DOCKER_COMPOSE) build

build: compose-build
	$(DOCKER_COMPOSE) run --rm $(DEV_SERVICE) bash -lc "make build-in-container"

test: compose-build
	$(DOCKER_COMPOSE) run --rm $(DEV_SERVICE) bash -lc "make test-in-container"

package: compose-build
	$(DOCKER_COMPOSE) run --rm $(DEV_SERVICE) bash -lc "make package-in-container"

init-in-container:
	@if [ ! -d "$(PGRX_HOME)/pg$(PG_MAJOR)" ]; then \
		cargo pgrx init --pg$(PG_MAJOR) $(PG_CONFIG); \
	fi

build-in-container: init-in-container
	cargo pgrx install --release --pg-config $(PG_CONFIG)
	@mkdir -p $(BUILD_DIR)
	@sharedir=$$($(PG_CONFIG) --sharedir); \
	pkglibdir=$$($(PG_CONFIG) --pkglibdir); \
	cp $$pkglibdir/$(EXT_NAME).so $(BUILD_DIR)/; \
	cp $$sharedir/extension/$(EXT_NAME).control $(BUILD_DIR)/; \
	cp $$sharedir/extension/$(EXT_NAME)--*.sql $(BUILD_DIR)/

test-in-container: init-in-container
	@pgdata=/tmp/pgrx-test-pgdata; \
	rm -rf $$pgdata; \
	mkdir -p $$pgdata; \
	chown -R postgres:postgres $$pgdata; \
	cargo pgrx test pg$(PG_MAJOR) --runas postgres --pgdata $$pgdata

package-in-container: init-in-container
	cargo pgrx package --release --pg-config $(PG_CONFIG)
	@mkdir -p $(BUILD_DIR)/pg$(PG_MAJOR)
	@pkgdir=$$(find target/release -maxdepth 1 -type d -name "$(EXT_NAME)-pg$(PG_MAJOR)*" | head -n 1); \
	test -n "$$pkgdir"; \
	cp -R $$pkgdir/* $(BUILD_DIR)/pg$(PG_MAJOR)/

clean:
	rm -rf $(BUILD_DIR)
