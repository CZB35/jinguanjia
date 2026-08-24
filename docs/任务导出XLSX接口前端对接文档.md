## 适用范围

本文说明两个导出接口：

- `GET /tasks/export`
- `POST /tasks/export/selected`

这两个接口都是任务列表导出接口，已经从旧的 `csv` 导出改成新的 `xlsx` 导出。

所有接口默认都挂在 `/api` 下。

## 1. 接口地址

```http
GET /tasks/export
```

```http
POST /tasks/export/selected
```

完整线上地址示例：

```text
https://service.ryds.cn/api/tasks/export
```

说明：

- 这才是“任务导出”接口
- 不是 `/notices/unread-count`

### 1.1 全量导出

```http
GET /tasks/export
```

用途：

- 按当前筛选条件导出整批任务

### 1.2 导出选中任务

```http
POST /tasks/export/selected
```

用途：

- 前端勾选若干任务后，只导出选中的任务

## 2. 当前行为

### 2.1 之前

之前导出的是：

- `csv`
- 英文表头
- 只能导出文字
- 现场图片只能做链接，不能直接进 Excel 表格

### 2.2 现在

现在导出的是：

- `xlsx`
- 中文表头
- 现场图片直接插入到 Excel 单元格
- 不是图片链接

## 3. 请求方式

请求头：

```http
Authorization: Bearer <access_token>
```

说明：

- 需要登录态
- 一般前端直接走文件下载即可

### 3.1 全量导出

```http
GET /tasks/export
```

### 3.2 导出选中任务

```http
POST /tasks/export/selected
Content-Type: application/json
```

请求体：

```json
{
  "task_ids": [1, 2, 3]
}
```

说明：

- `task_ids` 不能为空
- 只会导出这些任务
- 导出格式与全量导出保持一致

## 4. 返回结果

返回的是文件流，不是 JSON。

两个接口返回格式一致。

### 4.1 文件类型

```text
application/vnd.openxmlformats-officedocument.spreadsheetml.sheet
```

### 4.2 文件名

```text
tasks-export.xlsx
```

## 5. 表头说明

当前基础文本表头按客户要求固定为：

- `客户手机号`
- `客户姓名`
- `任务类型`
- `师傅姓名`
- `师傅手机号`
- `师傅评分`
- `门店`
- `经销商名称`
- `经销商评价`
- `客户评价`
- `客户地址`
- `师傅定位地址`

## 6. 现场图片列

### 6.1 当前规则

附件图片列不是固定一列，而是动态扩展。

例如：

- 如果这次导出的所有任务里，最大现场图片数是 `1`
  - 表头会有：`附件图片`
- 如果最大现场图片数是 `3`
  - 表头会有：
    - `附件图片1`
    - `附件图片2`
    - `附件图片3`

### 6.2 单条任务的处理方式

- 某条任务有几张现场图，就插入几张
- 某条任务没有现场图，对应列留空
- 插入的是压缩后的图片本体，不是 URL 链接

### 6.3 图片来源

后端导出时只取：

- `evidence` 表
- `evidence_type = image`

不会把普通附件图混进附件图片列。

## 7. 前端注意事项

### 7.1 不要再按 CSV 处理

现在前端不能再把这个接口当作 `.csv` 下载。

应该按 Excel 文件处理：

- 文件后缀：`.xlsx`
- 浏览器下载后直接打开 Excel

### 7.2 前端不需要自己拼图片链接

这个接口已经把现场图片直接嵌进 Excel 了。

前端只负责：

- 发起下载
- 接收文件流
- 保存文件

不需要再自己把图片 URL 塞进表格。

### 7.3 列表页推荐接法

#### 全量导出

直接调用：

```http
GET /tasks/export
```

#### 导出选中

勾选任务后调用：

```http
POST /tasks/export/selected
```

请求体直接传当前勾选任务主单 ID：

```json
{
  "task_ids": [101, 102, 103]
}
```

## 8. 后端实现说明

当前后端实现位置：

- 路由注册：`src/routes/mod.rs`
- 导出逻辑：`src/routes/tasks.rs`

其中：

- 全量导出：`GET /tasks/export`
- 导出选中：`POST /tasks/export/selected`

导出依赖：

```toml
rust_xlsxwriter = "0.95.0"
```

## 9. 上线环境注意事项

现场图片能否真正显示，除了数据库里有记录，还依赖部署机本地上传目录里有真实图片文件。

也就是说：

- 如果数据库有 `evidence` 记录
- 但服务器 `uploads/` 目录里没有对应图片文件

则 Excel 文件仍会导出成功，但对应图片单元格会为空。

## 10. 建议验收方式

建议前端 / 测试联调时这样验：

1. 调用全量导出：

```http
GET /tasks/export
```

或调用选中导出：

```http
POST /tasks/export/selected
```

2. 确认下载文件：

- 后缀是 `.xlsx`
- 文件名是 `tasks-export.xlsx`

3. 打开 Excel 后确认：

- 表头已经变成中文
- 有现场图片的任务行，图片直接显示在表格里
- 没现场图片的任务行，对应列为空
