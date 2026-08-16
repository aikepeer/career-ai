use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("file not found: {0}")]
    FileNotFound(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("csv: {0}")]
    Csv(#[from] csv::Error),

    #[error("xml: {0}")]
    Xml(#[from] quick_xml::Error),

    #[error("pdf: {0}")]
    Pdf(String),

    #[error("unsupported file type: {0}")]
    UnsupportedFormat(String),

    #[error("linkedin export missing required file: {0}")]
    LinkedInMissingFile(String),

    #[error("linkedin export schema changed — missing required column: {0}")]
    LinkedInMissingColumn(String),

    #[error("schema validation failed: {0}")]
    Validation(String),
}

pub type Result<T> = std::result::Result<T, ProfileError>;
