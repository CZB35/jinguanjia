SET NAMES utf8mb4;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'evidence' AND COLUMN_NAME = 'location_address'
  ),
  'SELECT 1',
  'ALTER TABLE `evidence` ADD COLUMN `location_address` VARCHAR(255) NULL COMMENT ''师傅现场定位地址'' AFTER `watermark_text`'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;
