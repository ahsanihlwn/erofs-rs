# erofs-rs

Native Rust toolkit for **EROFS** images: inspect, multi-threaded Android ROM extraction (`fs_config` / `file_contexts` / `mkfs_options`), pack (`none`, `lz4`, `lz4hc`, `deflate`, `zstd`, `lzma`), verify, **direct in-place/append editing without unpacking**, **in-place compaction**, AVB footer handling, Android Sparse support, and cross-partition symlink resolution.

## Build

```bash
cargo build --release
```

Binary output: `target/release/erofs-rs` (or `erofs-rs.exe` on Windows).

## CLI Quick Reference

All commands accept global `--json` (for scripts and GUI integration) and `--offset <BYTES>`.

```bash
# Detect filesystem & sparse status
erofs-rs detect product.img

# Inspect superblock, features, compressors, CRC32C, AVB footer
erofs-rs info product.img

# View all entries (flat, long, tree, or filtered)
erofs-rs list product.img --long
erofs-rs list product.img --tree
erofs-rs list product.img /etc --filter prop

# Inspect single inode metadata, xattrs, SELinux, capabilities, physical blocks
erofs-rs stat product.img /etc/build.prop

# Read a file directly from the image without unpacking
erofs-rs read product.img /etc/build.prop
erofs-rs read product.img /etc/build.prop --check-text --json

# Multi-threaded unpack + Android fs_config / file_contexts / mkfs_options
erofs-rs unpack product.img -o ./out --overwrite
erofs-rs unpack product.img -o ./out --config-only
erofs-rs unpack product.img -o ./out --target /etc --recursive

# Pack a directory into an EROFS image
erofs-rs pack ./out/product product_repack.img \
  --algo lz4hc --pcluster 16384 \
  --fs-config ./out/config/product_fs_config \
  --file-contexts ./out/config/product_file_contexts

# Verify CRC32C, dirent ordering, full decompression & optional SHA-256 xattr digests
erofs-rs verify product.img

# Direct edit WITHOUT unpacking (auto-refreshes superblock CRC32C & strips AVB if appending)
erofs-rs edit write product.img /etc/build.prop --from ./edited_build.prop --dry-run
erofs-rs edit write product.img /etc/build.prop --from ./edited_build.prop
erofs-rs edit chmod product.img /etc/build.prop 0644
erofs-rs edit selinux product.img /etc/build.prop u:object_r:system_file:s0
erofs-rs edit rm product.img /app/BloatApp --recursive

# Space usage & in-place compaction (reclaim holes & relocate metadata)
erofs-rs usage product.img
erofs-rs compact product.img --dry-run
erofs-rs compact product.img

# AVB footer inspection & stripping
erofs-rs avb inspect product.img
erofs-rs avb strip product.img

# Cross-partition Android symlink resolution
erofs-rs resolve system.img /product --mount product.img --mount vendor.img
```

## License

MIT License — see [LICENSE](LICENSE).
