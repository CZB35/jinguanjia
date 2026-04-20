-- 四专检核系统初始化迁移脚本
-- 数据库: MySQL 8.x

SET NAMES utf8mb4;

CREATE TABLE `user` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `mobile` VARCHAR(20) NOT NULL COMMENT '手机号',
  `password_hash` VARCHAR(255) NULL COMMENT '密码哈希',
  `name` VARCHAR(50) NOT NULL COMMENT '姓名',
  `email` VARCHAR(100) NULL COMMENT '邮箱',
  `avatar_url` VARCHAR(500) NULL COMMENT '头像地址',
  `status` VARCHAR(20) NOT NULL DEFAULT 'enabled' COMMENT '状态: enabled/disabled',
  `last_login_at` DATETIME NULL COMMENT '最后登录时间',
  `last_login_ip` VARCHAR(64) NULL COMMENT '最后登录IP',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  `updated_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP COMMENT '更新时间',
  `deleted_at` DATETIME NULL COMMENT '软删除时间',
  UNIQUE KEY `uk_user_mobile` (`mobile`),
  KEY `idx_user_status` (`status`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='系统用户表';

CREATE TABLE `role` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `role_code` VARCHAR(50) NOT NULL COMMENT '角色编码',
  `role_name` VARCHAR(50) NOT NULL COMMENT '角色名称',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  UNIQUE KEY `uk_role_code` (`role_code`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='角色表';

CREATE TABLE `user_role` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `user_id` BIGINT NOT NULL COMMENT '用户ID',
  `role_id` BIGINT NOT NULL COMMENT '角色ID',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  UNIQUE KEY `uk_user_role` (`user_id`, `role_id`),
  KEY `idx_user_role_user_id` (`user_id`),
  KEY `idx_user_role_role_id` (`role_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='用户角色关系表';

CREATE TABLE `store` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `store_name` VARCHAR(100) NOT NULL COMMENT '门店名称',
  `contact_name` VARCHAR(50) NULL COMMENT '联系人',
  `contact_mobile` VARCHAR(20) NULL COMMENT '联系电话',
  `province` VARCHAR(50) NULL COMMENT '省',
  `city` VARCHAR(50) NULL COMMENT '市',
  `district` VARCHAR(50) NULL COMMENT '区',
  `address` VARCHAR(255) NULL COMMENT '详细地址',
  `status` VARCHAR(20) NOT NULL DEFAULT 'enabled' COMMENT '状态: enabled/disabled',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  `updated_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP COMMENT '更新时间'
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='门店表';

CREATE TABLE `user_store` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `user_id` BIGINT NOT NULL COMMENT '用户ID',
  `store_id` BIGINT NOT NULL COMMENT '门店ID',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  UNIQUE KEY `uk_user_store` (`user_id`, `store_id`),
  KEY `idx_user_store_store_id` (`store_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='用户门店关系表';

CREATE TABLE `technician_skill` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `user_id` BIGINT NOT NULL COMMENT '技师用户ID',
  `service_type` VARCHAR(30) NOT NULL COMMENT '服务环节: measure/install/ironing/cleaning',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  UNIQUE KEY `uk_technician_skill` (`user_id`, `service_type`),
  KEY `idx_technician_skill_service_type` (`service_type`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='技师服务能力表';

CREATE TABLE `customer` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `name` VARCHAR(50) NOT NULL COMMENT '客户姓名',
  `mobile` VARCHAR(20) NOT NULL COMMENT '客户手机号',
  `source` VARCHAR(50) NULL COMMENT '客户来源',
  `remark` VARCHAR(255) NULL COMMENT '备注',
  `created_by` BIGINT NOT NULL COMMENT '创建人ID',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  `updated_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP COMMENT '更新时间',
  KEY `idx_customer_mobile` (`mobile`),
  KEY `idx_customer_name` (`name`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='客户表';

CREATE TABLE `customer_address` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `customer_id` BIGINT NOT NULL COMMENT '客户ID',
  `province` VARCHAR(50) NULL COMMENT '省',
  `city` VARCHAR(50) NULL COMMENT '市',
  `district` VARCHAR(50) NULL COMMENT '区',
  `address` VARCHAR(255) NOT NULL COMMENT '详细地址',
  `is_default` TINYINT NOT NULL DEFAULT 0 COMMENT '是否默认地址: 0否1是',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  KEY `idx_customer_address_customer_id` (`customer_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='客户地址表';

CREATE TABLE `task` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `task_no` VARCHAR(50) NOT NULL COMMENT '任务编号',
  `customer_id` BIGINT NOT NULL COMMENT '客户ID',
  `customer_address_id` BIGINT NOT NULL COMMENT '客户地址ID',
  `store_id` BIGINT NOT NULL COMMENT '门店ID',
  `dealer_user_id` BIGINT NOT NULL COMMENT '经销商用户ID',
  `current_technician_id` BIGINT NULL COMMENT '当前技师ID',
  `service_type` VARCHAR(30) NOT NULL COMMENT '服务环节: measure/install/ironing/cleaning',
  `priority` VARCHAR(20) NOT NULL DEFAULT 'normal' COMMENT '优先级: low/normal/high',
  `appointment_time` DATETIME NOT NULL COMMENT '预约时间',
  `task_desc` VARCHAR(500) NULL COMMENT '任务描述',
  `task_status` VARCHAR(30) NOT NULL COMMENT '任务状态',
  `review_status` VARCHAR(30) NOT NULL DEFAULT 'pending' COMMENT '评价状态: pending/reviewed',
  `score_status` VARCHAR(30) NOT NULL DEFAULT 'pending' COMMENT '评分状态: pending/scored',
  `is_overdue` TINYINT NOT NULL DEFAULT 0 COMMENT '是否逾期: 0否1是',
  `latest_assignment_id` BIGINT NULL COMMENT '最新分派记录ID',
  `latest_execution_id` BIGINT NULL COMMENT '最新执行记录ID',
  `source` VARCHAR(30) NOT NULL DEFAULT 'mini_program' COMMENT '任务来源: mini_program/pc_admin/oms',
  `created_by` BIGINT NOT NULL COMMENT '创建人ID',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  `updated_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP COMMENT '更新时间',
  `deleted_at` DATETIME NULL COMMENT '软删除时间',
  UNIQUE KEY `uk_task_no` (`task_no`),
  KEY `idx_task_store_status` (`store_id`, `task_status`),
  KEY `idx_task_technician_status` (`current_technician_id`, `task_status`),
  KEY `idx_task_customer_id` (`customer_id`),
  KEY `idx_task_appointment_time` (`appointment_time`),
  KEY `idx_task_store_service_status_time` (`store_id`, `service_type`, `task_status`, `appointment_time`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='任务主表';

CREATE TABLE `task_assignment` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `task_id` BIGINT NOT NULL COMMENT '任务ID',
  `from_technician_id` BIGINT NULL COMMENT '原技师ID',
  `to_technician_id` BIGINT NOT NULL COMMENT '新技师ID',
  `assign_type` VARCHAR(20) NOT NULL COMMENT '分派类型: assign/reassign',
  `assign_reason` VARCHAR(255) NULL COMMENT '分派说明或重派原因',
  `assigned_by` BIGINT NOT NULL COMMENT '分派人ID',
  `assigned_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '分派时间',
  KEY `idx_task_assignment_task_id` (`task_id`),
  KEY `idx_task_assignment_to_technician_id` (`to_technician_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='任务分派记录表';

CREATE TABLE `task_execution` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `task_id` BIGINT NOT NULL COMMENT '任务ID',
  `technician_id` BIGINT NOT NULL COMMENT '技师ID',
  `started_at` DATETIME NULL COMMENT '开始执行时间',
  `submitted_at` DATETIME NULL COMMENT '提交检核时间',
  `submit_remark` VARCHAR(255) NULL COMMENT '提交说明',
  `execution_status` VARCHAR(30) NOT NULL DEFAULT 'pending' COMMENT '执行状态: pending/executing/submitted/cancelled',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  `updated_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP COMMENT '更新时间',
  KEY `idx_task_execution_task_id` (`task_id`),
  KEY `idx_task_execution_technician_id` (`technician_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='任务执行记录表';

CREATE TABLE `evidence` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `task_id` BIGINT NOT NULL COMMENT '任务ID',
  `execution_id` BIGINT NOT NULL COMMENT '执行记录ID',
  `uploader_id` BIGINT NOT NULL COMMENT '上传人ID',
  `evidence_type` VARCHAR(20) NOT NULL COMMENT '实据类型: image/video',
  `file_name` VARCHAR(255) NOT NULL COMMENT '文件名',
  `file_url` VARCHAR(500) NOT NULL COMMENT '文件地址',
  `file_size` BIGINT NOT NULL COMMENT '文件大小(字节)',
  `mime_type` VARCHAR(100) NOT NULL COMMENT '文件MIME类型',
  `watermark_text` VARCHAR(500) NULL COMMENT '水印文本',
  `longitude` DECIMAL(10,6) NULL COMMENT '经度',
  `latitude` DECIMAL(10,6) NULL COMMENT '纬度',
  `shot_at` DATETIME NULL COMMENT '拍摄时间',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  KEY `idx_evidence_task_id` (`task_id`),
  KEY `idx_evidence_execution_id` (`execution_id`),
  KEY `idx_evidence_type` (`evidence_type`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='现场实据表';

CREATE TABLE `attachment` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `task_id` BIGINT NOT NULL COMMENT '任务ID',
  `uploader_id` BIGINT NOT NULL COMMENT '上传人ID',
  `attachment_type` VARCHAR(30) NOT NULL COMMENT '附件类型: task_attachment/score_import_file',
  `file_name` VARCHAR(255) NOT NULL COMMENT '文件名',
  `file_url` VARCHAR(500) NOT NULL COMMENT '文件地址',
  `file_size` BIGINT NOT NULL COMMENT '文件大小(字节)',
  `mime_type` VARCHAR(100) NOT NULL COMMENT '文件MIME类型',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  KEY `idx_attachment_task_id` (`task_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='任务附件表';

CREATE TABLE `dealer_review` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `task_id` BIGINT NOT NULL COMMENT '任务ID',
  `reviewer_id` BIGINT NOT NULL COMMENT '评价人ID',
  `star` TINYINT NOT NULL COMMENT '星级: 1-5',
  `tag_json` JSON NULL COMMENT '评价标签JSON',
  `content` VARCHAR(500) NULL COMMENT '评价内容',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  UNIQUE KEY `uk_dealer_review_task_id` (`task_id`),
  KEY `idx_dealer_review_reviewer_id` (`reviewer_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='经销商评价表';

CREATE TABLE `hq_score` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `task_id` BIGINT NOT NULL COMMENT '任务ID',
  `scorer_id` BIGINT NOT NULL COMMENT '评分人ID',
  `total_score` INT NOT NULL COMMENT '综合评分',
  `service_level` VARCHAR(30) NOT NULL COMMENT '服务等级: excellent/good/pass/improve',
  `score_remark` VARCHAR(500) NULL COMMENT '评分说明',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  UNIQUE KEY `uk_hq_score_task_id` (`task_id`),
  KEY `idx_hq_score_scorer_id` (`scorer_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='总部评分主表';

CREATE TABLE `hq_score_item` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `hq_score_id` BIGINT NOT NULL COMMENT '总部评分主表ID',
  `task_id` BIGINT NOT NULL COMMENT '任务ID',
  `service_type` VARCHAR(30) NOT NULL COMMENT '服务环节',
  `score` INT NOT NULL COMMENT '单项评分',
  `deduction_reason` VARCHAR(500) NULL COMMENT '扣分原因',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  KEY `idx_hq_score_item_hq_score_id` (`hq_score_id`),
  KEY `idx_hq_score_item_task_id` (`task_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='总部评分明细表';

CREATE TABLE `notice` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `receiver_id` BIGINT NOT NULL COMMENT '接收人ID',
  `notice_type` VARCHAR(30) NOT NULL COMMENT '通知类型: site/sms/wechat',
  `biz_type` VARCHAR(30) NOT NULL COMMENT '业务类型',
  `biz_id` BIGINT NOT NULL COMMENT '业务ID',
  `title` VARCHAR(100) NOT NULL COMMENT '标题',
  `content` VARCHAR(500) NOT NULL COMMENT '内容',
  `is_read` TINYINT NOT NULL DEFAULT 0 COMMENT '是否已读: 0否1是',
  `read_at` DATETIME NULL COMMENT '已读时间',
  `send_status` VARCHAR(20) NOT NULL DEFAULT 'pending' COMMENT '发送状态: pending/sent/failed',
  `sent_at` DATETIME NULL COMMENT '发送时间',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  KEY `idx_notice_receiver_id` (`receiver_id`),
  KEY `idx_notice_biz_type_biz_id` (`biz_type`, `biz_id`),
  KEY `idx_notice_receiver_status_time` (`receiver_id`, `send_status`, `created_at`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='通知表';

CREATE TABLE `audit_log` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `biz_type` VARCHAR(30) NOT NULL COMMENT '业务类型',
  `biz_id` BIGINT NOT NULL COMMENT '业务ID',
  `action` VARCHAR(50) NOT NULL COMMENT '操作动作',
  `operator_id` BIGINT NOT NULL COMMENT '操作人ID',
  `operator_role` VARCHAR(30) NOT NULL COMMENT '操作人角色',
  `before_value` JSON NULL COMMENT '变更前数据',
  `after_value` JSON NULL COMMENT '变更后数据',
  `remark` VARCHAR(500) NULL COMMENT '备注',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  KEY `idx_audit_log_biz_type_biz_id` (`biz_type`, `biz_id`),
  KEY `idx_audit_log_operator_id` (`operator_id`),
  KEY `idx_audit_log_operator_time` (`operator_id`, `created_at`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='操作审计日志表';

CREATE TABLE `system_config` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `config_key` VARCHAR(100) NOT NULL COMMENT '配置键',
  `config_value` JSON NOT NULL COMMENT '配置值',
  `remark` VARCHAR(255) NULL COMMENT '备注',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  `updated_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP COMMENT '更新时间',
  UNIQUE KEY `uk_system_config_key` (`config_key`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='系统配置表';

CREATE TABLE `permission` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `permission_code` VARCHAR(100) NOT NULL COMMENT '权限编码',
  `permission_name` VARCHAR(100) NOT NULL COMMENT '权限名称',
  `permission_type` VARCHAR(20) NOT NULL COMMENT '权限类型: menu/page/action',
  `parent_id` BIGINT NULL COMMENT '父权限ID',
  `sort_no` INT NOT NULL DEFAULT 0 COMMENT '排序号',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  UNIQUE KEY `uk_permission_code` (`permission_code`),
  KEY `idx_permission_parent_id` (`parent_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='权限表';

CREATE TABLE `role_permission` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `role_id` BIGINT NOT NULL COMMENT '角色ID',
  `permission_id` BIGINT NOT NULL COMMENT '权限ID',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  UNIQUE KEY `uk_role_permission` (`role_id`, `permission_id`),
  KEY `idx_role_permission_permission_id` (`permission_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='角色权限关系表';

CREATE TABLE `user_message_setting` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `user_id` BIGINT NOT NULL COMMENT '用户ID',
  `new_task_enabled` TINYINT NOT NULL DEFAULT 1 COMMENT '新任务提醒开关: 0关1开',
  `review_reminder_enabled` TINYINT NOT NULL DEFAULT 1 COMMENT '评价提醒开关: 0关1开',
  `sms_enabled` TINYINT NOT NULL DEFAULT 1 COMMENT '短信提醒开关: 0关1开',
  `wechat_enabled` TINYINT NOT NULL DEFAULT 1 COMMENT '微信提醒开关: 0关1开',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  `updated_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP COMMENT '更新时间',
  UNIQUE KEY `uk_user_message_setting_user_id` (`user_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='用户消息设置表';

CREATE TABLE `service_standard` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `service_type` VARCHAR(30) NOT NULL COMMENT '服务环节',
  `title` VARCHAR(100) NOT NULL COMMENT '标准标题',
  `content` TEXT NULL COMMENT '标准说明',
  `example_file_url` VARCHAR(500) NULL COMMENT '示例文件地址',
  `status` VARCHAR(20) NOT NULL DEFAULT 'enabled' COMMENT '状态: enabled/disabled',
  `version_no` VARCHAR(30) NULL COMMENT '标准版本号',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  `updated_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP COMMENT '更新时间',
  KEY `idx_service_standard_service_type_status` (`service_type`, `status`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='服务标准表';

CREATE TABLE `system_version` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `version_no` VARCHAR(30) NOT NULL COMMENT '版本号',
  `version_title` VARCHAR(100) NOT NULL COMMENT '版本标题',
  `changelog` TEXT NULL COMMENT '更新说明',
  `published_at` DATETIME NULL COMMENT '发布时间',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  UNIQUE KEY `uk_system_version_version_no` (`version_no`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='系统版本表';

CREATE TABLE `customer_review_link` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `task_id` BIGINT NOT NULL COMMENT '任务ID',
  `customer_id` BIGINT NOT NULL COMMENT '客户ID',
  `token` VARCHAR(100) NOT NULL COMMENT '外链令牌',
  `expired_at` DATETIME NOT NULL COMMENT '过期时间',
  `status` VARCHAR(20) NOT NULL DEFAULT 'pending' COMMENT '状态: pending/used/expired',
  `sent_at` DATETIME NULL COMMENT '发送时间',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  UNIQUE KEY `uk_customer_review_link_token` (`token`),
  KEY `idx_customer_review_link_task_id` (`task_id`),
  KEY `idx_customer_review_link_customer_id` (`customer_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='客户短信评价链接表';

CREATE TABLE `customer_review` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `task_id` BIGINT NOT NULL COMMENT '任务ID',
  `customer_id` BIGINT NOT NULL COMMENT '客户ID',
  `review_link_id` BIGINT NOT NULL COMMENT '评价链接ID',
  `star` TINYINT NOT NULL COMMENT '客户星级: 1-5',
  `tag_json` JSON NULL COMMENT '客户评价标签JSON',
  `content` VARCHAR(500) NULL COMMENT '客户评价内容',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  UNIQUE KEY `uk_customer_review_task_id` (`task_id`),
  KEY `idx_customer_review_customer_id` (`customer_id`),
  KEY `idx_customer_review_link_id` (`review_link_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='客户评价表';

CREATE TABLE `oms_config` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `base_url` VARCHAR(255) NOT NULL COMMENT 'OMS服务地址',
  `app_key` VARCHAR(100) NOT NULL COMMENT 'OMS应用标识',
  `app_secret` VARCHAR(255) NOT NULL COMMENT 'OMS密钥',
  `status` VARCHAR(20) NOT NULL DEFAULT 'enabled' COMMENT '状态: enabled/disabled',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  `updated_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP COMMENT '更新时间'
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='OMS配置表';

CREATE TABLE `oms_sync_log` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `biz_type` VARCHAR(30) NOT NULL COMMENT '业务类型',
  `biz_id` BIGINT NULL COMMENT '业务ID',
  `sync_direction` VARCHAR(20) NOT NULL COMMENT '同步方向: in/out',
  `request_payload` JSON NULL COMMENT '请求报文',
  `response_payload` JSON NULL COMMENT '响应报文',
  `sync_status` VARCHAR(20) NOT NULL COMMENT '同步状态: success/failed',
  `error_message` VARCHAR(500) NULL COMMENT '错误信息',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  KEY `idx_oms_sync_log_biz_type_biz_id` (`biz_type`, `biz_id`),
  KEY `idx_oms_sync_log_status` (`sync_status`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='OMS同步日志表';

INSERT INTO `role` (`role_code`, `role_name`) VALUES
('technician', '技师'),
('dealer', '经销商'),
('hq_reviewer', '总部检核人'),
('super_admin', '超级管理员');

INSERT INTO `system_config` (`config_key`, `config_value`, `remark`) VALUES
('task_status_enum', JSON_ARRAY('pending_assign', 'pending_execute', 'executing', 'pending_dealer_review', 'pending_hq_score', 'completed', 'closed'), '任务状态枚举'),
('service_type_enum', JSON_ARRAY('measure', 'install', 'ironing', 'cleaning'), '服务环节枚举'),
('task_evidence_image_limit', JSON_OBJECT('maxCount', 5), '单任务图片上传上限'),
('score_rule', JSON_OBJECT('min', 0, 'max', 100, 'integerOnly', true), '评分规则');
