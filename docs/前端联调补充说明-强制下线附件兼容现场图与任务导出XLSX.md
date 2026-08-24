## 适用范围

本文只说明昨天这一批后端改动，主要覆盖：

- 登录态强制失效 / 强制重新登录
- 任务附件接口兼容返回现场图片
- 任务导出从 CSV 改为 XLSX，并支持现场图片列

所有接口默认都挂在 `/api` 下。

## 1. 登录态强制失效

### 背景

为了避免“账号权限、密码、状态改了以后，旧 token 还能继续使用”，后端新增了用户会话版本号：

- 数据库字段：`user.session_version`
- 迁移文件：`migrations/0021_user_session_version.sql`

默认值：

- `1`

### 当前规则

当以下信息发生变更时，后端会自动把 `session_version + 1`：

- 后台账号密码被重置
- 用户自己修改密码
- 后台账号状态被启用 / 禁用
- 后台账号角色被修改
- 后台账号 `account` 被修改
- 后台账号门店权限被修改
- 经销商门店绑定被修改
- 师傅门店 / 服务能力被修改
- 账号被删除 / 批量删除
- 角色权限被修改时，该角色下已绑定用户也会一起失效

### 影响的 token

以下两种 token 都会失效：

- `access_token`
- `refresh_token`

也就是说：

- 旧 `access_token` 不能继续访问 `/auth/me`
- 旧 `refresh_token` 也不能继续调用 `/auth/refresh`

### 前端表现

当前典型返回文案：

```text
账号权限已变更，请重新登录
```

或者：

```text
登录已失效，请重新登录
```

前端建议统一处理为：

1. 清空本地登录态
2. 跳回登录页
3. 弹出提示“账号信息已变更，请重新登录”

### 影响接口

重点相关接口：

- `POST /auth/login/admin`
- `POST /auth/login/password`
- `POST /auth/login/code`
- `POST /auth/refresh`
- `GET /auth/me`

## 2. 任务附件接口兼容返回现场图片

### 之前的问题

有些页面把“现场图片 / 环境记录”也调成了：

```http
GET /tasks/{id}/attachments
```

但历史上这个接口只查普通附件，因此会出现“页面看不到现场图片”的情况。

### 当前兼容行为

现在：

```http
GET /tasks/{id}/attachments
```

已经兼容返回两类数据：

- 普通任务附件
- 现场实据 / 现场图片 / 现场视频

### 返回结构补充

返回项新增字段：

```json
{
  "id": 7,
  "source": "evidence",
  "file_name": "1.jpg",
  "file_url": "/uploads/evidences/images/xxx.jpg",
  "file_size": 227639,
  "mime_type": "image/jpeg"
}
```

`source` 含义：

- `attachment`：普通附件
- `evidence`：现场实据

### 前端建议

如果页面只是“要展示文件”，直接用：

```http
GET /tasks/{id}/attachments
```

如果页面本身是“任务环节记录 / 现场图片主列表”，仍建议优先用：

```http
GET /tasks/{id}/evidences
```

更清晰。

## 3. 任务导出改为 XLSX

### 之前行为

之前：

```http
GET /tasks/export
```

导出的是：

- `csv`
- 英文字段表头
- 只能导出文字，不能真正嵌入图片

### 当前行为

现在：

```http
GET /tasks/export
```

导出的是：

- `xlsx`
- 中文表头
- 支持现场图片列

下载文件名：

```text
tasks-export.xlsx
```

响应类型：

```text
application/vnd.openxmlformats-officedocument.spreadsheetml.sheet
```

### 当前表头

基础列已改成中文，例如：

- `任务ID`
- `任务编号`
- `服务类型`
- `服务项目`
- `任务状态`
- `展示状态`
- `评价状态`
- `评分状态`
- `预约时间`
- `客户姓名`
- `客户手机号`
- `门店名称`
- `经销商`
- `技师姓名`
- `技师手机号`
- `是否逾期`
- `任务标签`

### 现场图片列

现场图片列是动态生成的。

例如：

- 某次导出最大图片数为 1，则表头有：`现场图片1`
- 某次导出最大图片数为 3，则表头有：`现场图片1`、`现场图片2`、`现场图片3`

规则：

- 某条任务有几张现场图，就插入几张
- 某条任务没有现场图，对应单元格留空
- 图片不是链接，而是直接嵌入 Excel 单元格

### 图片来源

导出时只取：

- `evidence` 表
- `evidence_type = image`

不会把普通附件图片混进“现场图片列”。

### 注意事项

Excel 文件里能否真正看到图片，除了数据库里有记录，还依赖服务器本地上传目录里存在真实文件。

也就是说：

- 如果数据库有 `evidence` 记录
- 但部署机 `uploads/` 下没有对应图片文件

则导出的 Excel 仍会成功，但该图片单元格会为空。

## 4. 上线注意事项

### 依赖新增

本次新增了 Excel 导出依赖：

```toml
rust_xlsxwriter = "0.95.0"
```

如果线上编译报错：

```text
unresolved import `rust_xlsxwriter`
```

通常说明线上代码还没有同步到包含该依赖的最新 `Cargo.toml`。

### 必做项

上线前请确认：

1. 已拉到最新代码
2. `Cargo.toml` 包含 `rust_xlsxwriter = "0.95.0"`
3. `Cargo.lock` 已同步
4. 已执行数据库迁移：

```text
migrations/0021_user_session_version.sql
```

### 建议回归接口

#### 1. 健康检查

```http
GET /health
```

#### 2. 后台登录

```http
POST /auth/login/admin
```

#### 3. 任务附件兼容现场图

```http
GET /tasks/10/attachments?page=1&page_size=20
```

建议确认返回里存在：

```json
{
  "source": "evidence"
}
```

#### 4. 任务导出

```http
GET /tasks/export
```

建议确认：

- 下载文件后缀为 `.xlsx`
- 表头为中文
- 有现场图的任务能在 Excel 里看到嵌入图片

#### 5. 强制下线

建议验证：

1. 用某账号先登录
2. 后台重置该账号密码
3. 再拿旧 `access_token` 调 `/auth/me`
4. 再拿旧 `refresh_token` 调 `/auth/refresh`

预期：

- 都不能再成功
- 会返回“请重新登录”类文案

## 5. 提交信息

本批改动提交号：

```text
27b4387
```

提交说明：

```text
feat: enforce session invalidation and xlsx task export
```
