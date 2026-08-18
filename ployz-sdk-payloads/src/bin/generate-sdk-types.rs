fn main() {
    let root = ployz_sdk_payloads::sdk_package_root();
    ployz_sdk_payloads::write_generated(&root).expect("write generated @ployz/sdk artifacts");
}
