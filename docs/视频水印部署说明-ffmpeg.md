## 适用范围

本文只说明视频水印依赖的 `ffmpeg` 如何部署。

## 1. 当前后端查找顺序

视频上传打水印时，后端会按下面顺序找 `ffmpeg`：

1. 先读环境变量 `FFMPEG_PATH`
2. 再找程序同目录下的 `ffmpeg.exe`
3. 最后尝试系统环境变量里的 `ffmpeg`

## 2. 最省事的部署方式

推荐直接使用：

- 把 `ffmpeg.exe` 和后端程序放在同一个目录

例如：

```text
kgj-backend.exe
ffmpeg.exe
```

这样后端启动后会自动优先识别同目录下的 `ffmpeg.exe`。

## 3. 也支持环境变量

如果不想把 `ffmpeg.exe` 放在程序目录，也可以显式配置：

```text
FFMPEG_PATH=D:\deploy\tools\ffmpeg.exe
```

## 4. 上线前建议检查

1. 确认 `ffmpeg.exe` 已随程序一起部署，或者 `FFMPEG_PATH` 已配置
2. 用一条执行中的任务上传一个现场视频
3. 确认视频上传成功
4. 确认下载后的视频已带水印

## 5. 如果没配会怎样

如果后端找不到 `ffmpeg`，视频上传会直接失败，不会出现“上传成功但没水印”的情况。

报错文案大意为：

```text
服务端未找到可用的ffmpeg，优先检查FFMPEG_PATH，其次确认程序同目录是否存在ffmpeg.exe
```
