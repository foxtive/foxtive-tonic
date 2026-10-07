use tonic::Code;

/// A trait that maps application-level response codes to gRPC status codes.
///
/// Implementors provide a string code, a gRPC status, and a success indicator.
/// The default [`success`](ResponseCodeContract::success) method considers Ok
/// status codes as successful.
pub trait ResponseCodeContract: Clone {
    /// Returns the application-specific response code string (e.g. "000").
    fn code(&self) -> &str;

    /// Returns the corresponding gRPC status code.
    fn status(&self) -> Code;

    /// Returns `true` if the gRPC status indicates success.
    fn success(&self) -> bool {
        self.status() == Code::Ok
    }

    /// Creates a response code from its string representation.
    fn from_code(code: &str) -> Self;

    /// Creates a response code from a gRPC status code.
    fn from_status(status: Code) -> Self;
}
