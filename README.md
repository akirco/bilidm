# bilidm

一个用 Rust 编写的命令行工具，用于从哔哩哔哩（B站）中获取和可视化播放[视频弹幕](https://raw.githubusercontent.com/pskdje/bilibili-API-collect/refs/heads/main/docs/danmaku/danmaku_proto.md)。

## 安装

```bash
git clone https://github.com/akirco/bilidm
cd bilidm
cargo build --release
mv target/releases/bilidm ~/.local/bin
```

## 使用

```bash
cargo run -- BV1WYXDB7EPm

cargo run -- "https://www.bilibili.com/video/BV1WYXDB7EPm"
```

## 许可证

MIT

## 注意

- 本工具仅供学习和研究使用
- 请遵守 B 站的服务条款和 API 使用规范
- 如果遇到 API 频率限制，工具可能无法正常工作
