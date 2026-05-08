-- 微信服务号通知基础表结构

SET NAMES utf8mb4;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'user' AND COLUMN_NAME = 'wechat_openid'
  ),
  'SELECT 1',
  'ALTER TABLE `user` ADD COLUMN `wechat_openid` VARCHAR(128) NULL COMMENT ''微信服务号openid'' AFTER `avatar_url`'
);
PREPARE stmt FROM @ddl; EXECUTE stmt; DEALLOCATE PREPARE stmt;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'user' AND COLUMN_NAME = 'wechat_unionid'
  ),
  'SELECT 1',
  'ALTER TABLE `user` ADD COLUMN `wechat_unionid` VARCHAR(128) NULL COMMENT ''微信unionid'' AFTER `wechat_openid`'
);
PREPARE stmt FROM @ddl; EXECUTE stmt; DEALLOCATE PREPARE stmt;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'user' AND COLUMN_NAME = 'wechat_subscribe'
  ),
  'SELECT 1',
  'ALTER TABLE `user` ADD COLUMN `wechat_subscribe` TINYINT NOT NULL DEFAULT 0 COMMENT ''是否关注微信服务号:0否1是'' AFTER `wechat_unionid`'
);
PREPARE stmt FROM @ddl; EXECUTE stmt; DEALLOCATE PREPARE stmt;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'user' AND COLUMN_NAME = 'wechat_bind_at'
  ),
  'SELECT 1',
  'ALTER TABLE `user` ADD COLUMN `wechat_bind_at` DATETIME NULL COMMENT ''微信绑定时间'' AFTER `wechat_subscribe`'
);
PREPARE stmt FROM @ddl; EXECUTE stmt; DEALLOCATE PREPARE stmt;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'user' AND COLUMN_NAME = 'wechat_subscribe_updated_at'
  ),
  'SELECT 1',
  'ALTER TABLE `user` ADD COLUMN `wechat_subscribe_updated_at` DATETIME NULL COMMENT ''微信关注状态更新时间'' AFTER `wechat_bind_at`'
);
PREPARE stmt FROM @ddl; EXECUTE stmt; DEALLOCATE PREPARE stmt;

CREATE TABLE IF NOT EXISTS `notice_delivery` (
  `id` BIGINT PRIMARY KEY AUTO_INCREMENT COMMENT '主键ID',
  `notice_id` BIGINT NOT NULL COMMENT '通知主表ID',
  `channel` VARCHAR(20) NOT NULL COMMENT '投递渠道: site/wechat/sms',
  `receiver_id` BIGINT NOT NULL COMMENT '接收人ID',
  `receiver_openid` VARCHAR(128) NULL COMMENT '微信openid',
  `biz_type` VARCHAR(50) NOT NULL COMMENT '业务类型',
  `biz_id` BIGINT NOT NULL COMMENT '业务ID',
  `template_code` VARCHAR(100) NULL COMMENT '模板编码',
  `payload_json` JSON NULL COMMENT '模板入参JSON',
  `status` VARCHAR(20) NOT NULL DEFAULT 'pending' COMMENT '状态: pending/processing/sent/failed/cancelled',
  `attempt_count` INT NOT NULL DEFAULT 0 COMMENT '尝试次数',
  `next_retry_at` DATETIME NULL COMMENT '下次重试时间',
  `provider_message_id` VARCHAR(100) NULL COMMENT '供应商消息ID',
  `error_message` VARCHAR(500) NULL COMMENT '错误信息',
  `created_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP COMMENT '创建时间',
  `sent_at` DATETIME NULL COMMENT '发送完成时间',
  UNIQUE KEY `uk_notice_delivery_notice_channel` (`notice_id`, `channel`),
  KEY `idx_notice_delivery_status_retry` (`status`, `next_retry_at`),
  KEY `idx_notice_delivery_receiver_channel` (`receiver_id`, `channel`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='通知投递表';
