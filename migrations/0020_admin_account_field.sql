SET NAMES utf8mb4;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'user' AND COLUMN_NAME = 'account'
  ),
  'SELECT 1',
  'ALTER TABLE `user` ADD COLUMN `account` VARCHAR(50) NULL COMMENT ''后台登录账号'' AFTER `mobile`'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.STATISTICS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'user' AND INDEX_NAME = 'uk_user_account'
  ),
  'SELECT 1',
  'ALTER TABLE `user` ADD UNIQUE KEY `uk_user_account` (`account`)'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

UPDATE `user`
SET `account` = `mobile`
WHERE (`account` IS NULL OR `account` = '')
  AND `deleted_at` IS NULL;
