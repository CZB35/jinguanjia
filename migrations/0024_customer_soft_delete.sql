SET NAMES utf8mb4;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'customer' AND COLUMN_NAME = 'deleted_at'
  ),
  'SELECT 1',
  'ALTER TABLE `customer` ADD COLUMN `deleted_at` DATETIME NULL COMMENT ''软删除时间'' AFTER `updated_at`'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.STATISTICS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'customer' AND INDEX_NAME = 'idx_customer_deleted_at'
  ),
  'SELECT 1',
  'ALTER TABLE `customer` ADD KEY `idx_customer_deleted_at` (`deleted_at`)'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;
