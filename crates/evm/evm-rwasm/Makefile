FEATURES =

TARGET=wasm32-unknown-unknown
PROFILE=release

all: wat_file

wat_file:
	set -ex
	cargo b --profile ${PROFILE} --target=${TARGET} --features="${FEATURES}" -v
	mkdir -p bin
	wasm2wat ../../target/${TARGET}/${PROFILE}/fluentbase_rwasm_evm_translator.wasm > ./bin/$@.wat

clean:
	cargo clean

dep_tree: wat_file
	OPTIONS="--manifest-path Cargo.toml" #--no-default-features
	cargo tree ${OPTIONS} > ./bin/cargo-dep-tree.txt