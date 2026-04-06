# bilidm(开发中...)

一个用 Rust 编写的命令行工具，用于从哔哩哔哩（B站）中获取和可视化播放[视频弹幕](https://raw.githubusercontent.com/pskdje/bilibili-API-collect/refs/heads/main/docs/danmaku/danmaku_proto.md)。

## 功能

- 弹幕
- AI字幕 [需获取Cookie,`代码337行`]
- 音频播放

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

## 参考

- 字幕

```js
//https://github.com/Ciender/Bilibili_CC_Subtitles_AISummary/blob/main/main.js
async fetchSubtitleList() {
            const info = this.getEpInfo();
            if (!info || !info.cid) throw new Error("无法获取视频CID");

            if (this.cid !== info.cid) {
                this.cachedSubs = {};
            }

            this.cid = info.cid; this.aid = info.aid; this.bvid = info.bvid;
            const apiUrl = `https://api.bilibili.com/x/player/wbi/v2?cid=${this.cid}&aid=${this.aid}`;

            return new Promise((resolve, reject) => {
                GM_xmlhttpRequest({
                    method: 'GET', url: apiUrl, withCredentials: true,
                    onload: (res) => {
                        try {
                            const json = JSON.parse(res.responseText);
                            if (json.code === 0 && json.data && json.data.subtitle) {
                                this.subtitleInfo = json.data.subtitle;
                                resolve(this.subtitleInfo);
                            } else resolve({ subtitles: [] });
                        } catch (e) { reject(e); }
                    },
                    onerror: (e) => reject(e)
                });
            });
        },
        async fetchSubtitleContent(lan) {
            if (this.cachedSubs[lan]) return this.cachedSubs[lan];
            const subItem = this.subtitleInfo.subtitles.find(s => s.lan === lan);
            if (!subItem) throw new Error(`未找到语言 ${lan} 的字幕`);
            const url = subItem.subtitle_url.startsWith('//') ? 'https:' + subItem.subtitle_url : subItem.subtitle_url;
            return new Promise((resolve, reject) => {
                GM_xmlhttpRequest({
                    method: 'GET', url: url,
                    onload: (res) => {
                        try {
                            const json = JSON.parse(res.responseText);
                            this.cachedSubs[lan] = json;
                            resolve(json);
                        } catch (e) { reject(e); }
                    },
                    onerror: reject
                });
            });
        }
    };
```

- [audio](https://raw.githubusercontent.com/jingfelix/BiliFM/refs/heads/main/src/bilifm/audio.py)

## 注意

- 本工具仅供学习和研究使用
- 请遵守 B 站的服务条款和 API 使用规范
- 如果遇到 API 频率限制，工具可能无法正常工作
