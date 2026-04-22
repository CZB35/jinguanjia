# 金管家 Rust 后端

金管家项目后端服务，覆盖以下 4 个端：

1. 经销商端
2. 师傅端
3. 后台端
4. 客户短信评价端

## 技术栈

1. Web 框架：`axum`
2. ORM：`rbatis`
3. 数据库：`MySQL`
4. 驱动：`rbdc-mysql`
5. 认证：`JWT`
6. 异步运行时：`tokio`

## 当前能力

已完成：

1. 登录认证与角色切换
2. 经销商建单、分派、评价
3. 师傅接单、执行、上传实据、提交任务
4. 总部检核、评分、统计
5. 客户短信评价
6. 用户、客户、师傅、门店管理
7. 权限、系统配置、版本、OMS 基础接口
8. 文件上传、附件下载、批量下载

## 目录说明

```text
docs/         四个端的接口文档
migrations/   MySQL 初始化、种子数据、增量迁移脚本
src/models/   ORM 模型
src/routes/   路由与接口实现
src/db/       数据库兼容查询层
src/storage.rs 文件存储与路径处理
```

## 本地运行

1. 复制 `.env.example` 为 `.env`
2. 修改 `DATABASE_URL`
3. 修改 `JWT_SECRET`
4. 确认 MySQL 数据库已初始化

启动：

```bash
cargo run --release
```

默认服务端口：`3001`

健康检查：

```bash
GET /api/health
```

## 数据库脚本

主要脚本：

1. `migrations/0001_init_mysql.sql` 初始化表结构
2. `migrations/0002_seed_data.sql` 基础种子数据
3. `migrations/0003_task_service_items.sql` 多服务环节子表
4. `migrations/0004_perfect_fields.sql` 完美版字段补充
5. `migrations/0005_customer_review_images.sql` 客户评价图片表
6. `migrations/0006_cleanup_bad_test_data_and_seed_demo.sql` 清理坏数据并补演示数据
7. `migrations/0007_default_agreements.sql` 默认协议数据
8. `migrations/0008_customer_primary_store_and_store_crud.sql` 客户主门店字段与门店维护增强

## 接口文档

四个端的接口文档位于：

1. `docs/经销商端接口文档.md`
2. `docs/师傅端接口文档.md`
3. `docs/后台端接口文档.md`
4. `docs/客户短信评价端接口文档.md`

## 默认账号

管理员：

1. 手机号：`13800000000`
2. 密码：`Admin@123456`

演示账号：

1. 经销商：`13600000001`
2. 师傅：`13600000002`
3. 密码：`Admin@123456`
