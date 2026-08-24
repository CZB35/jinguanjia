# 阿里云 OSS 图片存储部署说明

## 目标

后端上传文件从服务器本地 `uploads/` 切换到阿里云 OSS。接口返回仍保持 `/uploads/...`，前端不需要改上传、预览、导出调用方式。

## 配置项

在服务器项目 `.env` 增加：

```env
STORAGE_BACKEND=oss
OSS_ENDPOINT=https://oss-cn-hangzhou.aliyuncs.com
OSS_BUCKET=你的Bucket名称
OSS_ACCESS_KEY_ID=你的AccessKeyId
OSS_ACCESS_KEY_SECRET=你的AccessKeySecret
```

说明：
- `STORAGE_BACKEND=local` 或不填时，仍使用本地 `UPLOAD_DIR`。
- `OSS_ENDPOINT` 填地域 endpoint，不要带 bucket 名。
- `OSS_BUCKET` 建议使用私有 Bucket；后端会用签名请求读写 OSS。
- 接口返回仍是 `/uploads/evidences/images/xxx.jpg` 这类路径，后端会代理读取 OSS 文件。

## 已覆盖功能

- 现场图片、视频、任务附件、客户评价图片、头像、项目设置图片上传。
- `/uploads/{path}` 文件访问。
- 附件下载、批量下载附件。
- 任务列表/环节记录导出中插入现场图片。
- 删除附件、删除现场图片、删除客户评价图片。
- 图片/视频水印刷新时重新读取并覆盖 OSS 对象。

## 发布步骤

```bash
cd /www/wwwroot/jinguanjia
git pull
vi .env
cargo build --release
sudo systemctl restart kgj-backend
sudo systemctl status kgj-backend -l
```

## 兼容旧文件

切到 OSS 后，新上传文件会写入 OSS。旧的本地 `uploads/` 文件仍可作为回退读取，但建议后续把服务器 `uploads/` 目录按原相对路径同步到 OSS，避免服务器迁移或清理后旧图片丢失。

## 验证

1. 上传一张项目设置图片或现场图片。
2. 确认接口返回 `file_url` 为 `/uploads/...`。
3. 浏览器打开 `https://service.ryds.cn/uploads/...` 能显示图片。
4. 测试环节记录导出，确认 Excel 中现场图片能插入。
