CREATE TABLE scraping_settings (
    singleton INT PRIMARY KEY CHECK(singleton = 1),
    config LONGTEXT NOT NULL
) ENGINE=InnoDB;
