SET NAMES utf8mb4;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'user_role' AND COLUMN_NAME = 'status'
  ),
  'SELECT 1',
  'ALTER TABLE `user_role` ADD COLUMN `status` VARCHAR(20) NOT NULL DEFAULT ''enabled'' COMMENT ''身份状态: enabled/disabled'' AFTER `role_id`'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

UPDATE `user_role`
SET `status` = 'enabled'
WHERE `status` IS NULL OR `status` = '';
