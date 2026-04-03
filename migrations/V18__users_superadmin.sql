ALTER TABLE users
ADD COLUMN is_superadmin BOOLEAN NOT NULL DEFAULT FALSE;

UPDATE users
SET is_superadmin = TRUE
WHERE role = 'admin';
