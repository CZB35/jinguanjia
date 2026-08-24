SET NAMES utf8mb4;

-- This migration intentionally fails if historical duplicate mobile values exist.
-- Merge duplicate customer references before retrying; do not delete customer rows directly.

SET @ddl = IF(
  EXISTS(
    SELECT 1 FROM INFORMATION_SCHEMA.STATISTICS
    WHERE TABLE_SCHEMA = DATABASE()
      AND TABLE_NAME = 'customer'
      AND INDEX_NAME = 'uk_customer_mobile'
  ),
  'SELECT 1',
  'ALTER TABLE `customer` ADD UNIQUE KEY `uk_customer_mobile` (`mobile`)'
);
PREPARE stmt FROM @ddl;
EXECUTE stmt;
DEALLOCATE PREPARE stmt;
