SET NAMES utf8mb4;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'user' AND COLUMN_NAME = 'session_version'
  ),
  'SELECT 1',
  'ALTER TABLE `user` ADD COLUMN `session_version` BIGINT NOT NULL DEFAULT 1 COMMENT ''登录会话版本'' AFTER `status`'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

UPDATE `user`
SET `session_version` = 1
WHERE `session_version` IS NULL;
