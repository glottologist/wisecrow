SELECT setval(
    pg_get_serial_sequence('users', 'id'),
    GREATEST(
        (SELECT last_value FROM users_id_seq),
        (SELECT COALESCE(MAX(id), 1) FROM users)
    ),
    true
);
