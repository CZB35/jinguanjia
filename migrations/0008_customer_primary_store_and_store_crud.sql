SET NAMES utf8mb4;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'customer' AND COLUMN_NAME = 'primary_store_id'
  ),
  'SELECT 1',
  'ALTER TABLE `customer` ADD COLUMN `primary_store_id` BIGINT NULL COMMENT ''主门店ID'' AFTER `remark`'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.STATISTICS
    WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'customer' AND INDEX_NAME = 'idx_customer_primary_store_id'
  ),
  'SELECT 1',
  'ALTER TABLE `customer` ADD KEY `idx_customer_primary_store_id` (`primary_store_id`)'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;

UPDATE `customer` c
JOIN (
  SELECT t.customer_id, CAST(SUBSTRING_INDEX(GROUP_CONCAT(t.store_id ORDER BY t.id DESC), ',', 1) AS UNSIGNED) AS latest_store_id
  FROM `task` t
  GROUP BY t.customer_id
) x ON x.customer_id = c.id
SET c.primary_store_id = x.latest_store_id
WHERE c.primary_store_id IS NULL;
