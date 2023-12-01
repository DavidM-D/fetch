//! Result and execution types from results of RPC calls to the network.

use near_gas::NearGas;
use near_primitives::borsh;
use near_primitives::views::{
    ExecutionOutcomeWithIdView, ExecutionStatusView, FinalExecutionOutcomeView,
    FinalExecutionStatus, SignedTransactionView,
};

use base64::{engine::general_purpose, Engine as _};

use crate::error::Result;
use crate::Error;

/// Execution related info as a result of performing a successful transaction
/// execution on the network. This value can be converted into the returned
/// value of the transaction via [`ExecutionSuccess::json`] or [`ExecutionSuccess::borsh`]
pub type ExecutionSuccess = ExecutionResult<Value>;

/// The transaction/receipt details of a transaction execution. This object
/// can be used to retrieve data such as logs and gas burnt per transaction
/// or receipt.
#[derive(PartialEq, Eq, Clone, Debug)]
#[non_exhaustive]
pub struct ExecutionDetails {
    /// Original signed transaction.
    pub transaction: SignedTransactionView,
    /// The execution outcome of the signed transaction.
    pub transaction_outcome: ExecutionOutcomeWithIdView,
    /// The execution outcome of receipts.
    pub receipts_outcome: Vec<ExecutionOutcomeWithIdView>,
}

impl ExecutionDetails {
    /// Returns just the transaction outcome.
    pub fn outcome(&self) -> &ExecutionOutcomeWithIdView {
        &self.transaction_outcome
    }

    /// Grab all outcomes after the execution of the transaction. This includes outcomes
    /// from the transaction and all the receipts it generated.
    pub fn outcomes(&self) -> Vec<&ExecutionOutcomeWithIdView> {
        let mut outcomes = vec![self.outcome()];
        outcomes.extend(self.receipt_outcomes());
        outcomes
    }

    /// Grab all outcomes after the execution of the transaction. This includes outcomes
    /// only from receipts generated by this transaction.
    pub fn receipt_outcomes(&self) -> &[ExecutionOutcomeWithIdView] {
        &self.receipts_outcome
    }

    /// Grab all outcomes that did not succeed the execution of this transaction. This
    /// will also include the failures from receipts as well.
    pub fn failures(&self) -> Vec<&ExecutionOutcomeWithIdView> {
        let mut failures = Vec::new();
        if matches!(
            self.transaction_outcome.outcome.status,
            ExecutionStatusView::Failure(_)
        ) {
            failures.push(&self.transaction_outcome);
        }
        failures.extend(self.receipt_failures());
        failures
    }

    /// Just like `failures`, grab only failed receipt outcomes.
    pub fn receipt_failures(&self) -> Vec<&ExecutionOutcomeWithIdView> {
        self.receipt_outcomes()
            .iter()
            .filter(|receipt| matches!(receipt.outcome.status, ExecutionStatusView::Failure(_)))
            .collect()
    }

    /// Grab all logs from both the transaction and receipt outcomes.
    pub fn logs(&self) -> Vec<&str> {
        self.outcomes()
            .into_iter()
            .flat_map(|outcome| &outcome.outcome.logs)
            .map(String::as_str)
            .collect()
    }
}

/// The result after evaluating the status of an execution. This can be [`ExecutionSuccess`]
/// for successful executions or a [`ExecutionFailure`] for failed ones.
#[derive(PartialEq, Eq, Debug, Clone)]
#[non_exhaustive]
pub struct ExecutionResult<T> {
    /// Total gas burnt by the execution
    pub total_gas_burnt: NearGas,

    /// Value returned from an execution. This is a base64 encoded str for a successful
    /// execution or a `TxExecutionError` if a failed one.
    pub value: T,

    /// Additional details related to the execution.
    pub details: ExecutionDetails,
}

/// Execution related info found after performing a transaction. Can be converted
/// into [`ExecutionSuccess`] or [`ExecutionFailure`] through [`into_result`]
///
/// [`into_result`]: crate::result::ExecutionFinalResult::into_result
#[derive(PartialEq, Eq, Clone, Debug)]
// #[must_use = "use `into_result()` to handle potential execution errors"]
pub struct ExecutionFinalResult {
    status: FinalExecutionStatus,
    pub details: ExecutionDetails,
}

impl ExecutionFinalResult {
    pub(crate) fn from_view(view: FinalExecutionOutcomeView) -> Self {
        Self {
            status: view.status,
            details: ExecutionDetails {
                transaction: view.transaction,
                transaction_outcome: view.transaction_outcome,
                receipts_outcome: view.receipts_outcome,
            },
        }
    }

    /// Converts this object into a [`Result`] holding either [`ExecutionSuccess`] or [`ExecutionFailure`].
    pub fn into_result(self) -> Result<ExecutionSuccess> {
        let total_gas_burnt = self.total_gas_burnt();
        match self.status {
            FinalExecutionStatus::SuccessValue(value) => Ok(ExecutionResult {
                total_gas_burnt,
                value: Value::from_string(general_purpose::STANDARD.encode(value)),
                details: self.details,
            }),
            FinalExecutionStatus::Failure(tx_error) => Err(Error::TxExecution(Box::new(tx_error))),
            _ => unreachable!(),
        }
    }

    pub fn total_gas_burnt(&self) -> NearGas {
        NearGas::from_gas(
            self.details.transaction_outcome.outcome.gas_burnt
                + self
                    .details
                    .receipts_outcome
                    .iter()
                    .map(|t| t.outcome.gas_burnt)
                    .sum::<u64>(),
        )
    }

    /// Returns the contained Ok value, consuming the self value.
    ///
    /// Because this function may panic, its use is generally discouraged. Instead, prefer
    /// to call into [`into_result`] then pattern matching and handle the Err case explicitly.
    ///
    /// [`into_result`]: crate::result::ExecutionFinalResult::into_result
    pub fn unwrap(self) -> ExecutionSuccess {
        self.into_result().unwrap()
    }

    /// Deserialize an instance of type `T` from bytes of JSON text sourced from the
    /// execution result of this call. This conversion can fail if the structure of
    /// the internal state does not meet up with [`serde::de::DeserializeOwned`]'s
    /// requirements.
    pub fn json<T: serde::de::DeserializeOwned>(self) -> Result<T> {
        self.into_result()?.json()
    }

    /// Deserialize an instance of type `T` from bytes sourced from the execution
    /// result. This conversion can fail if the structure of the internal state does
    /// not meet up with [`borsh::BorshDeserialize`]'s requirements.
    pub fn borsh<T: borsh::BorshDeserialize>(self) -> Result<T> {
        self.into_result()?.borsh()
    }

    /// Grab the underlying raw bytes returned from calling into a contract's function.
    /// If we want to deserialize these bytes into a rust datatype, use [`ExecutionResult::json`]
    /// or [`ExecutionResult::borsh`] instead.
    pub fn raw_bytes(self) -> Result<Vec<u8>> {
        self.into_result()?.raw_bytes()
    }

    /// Checks whether the transaction was successful. Returns true if
    /// the transaction has a status of [`FinalExecutionStatus::SuccessValue`].
    pub fn is_success(&self) -> bool {
        matches!(self.status(), FinalExecutionStatus::SuccessValue(_))
    }

    /// Checks whether the transaction has failed. Returns true if
    /// the transaction has a status of [`FinalExecutionStatus::Failure`].
    pub fn is_failure(&self) -> bool {
        matches!(self.status(), FinalExecutionStatus::Failure(_))
    }

    /// Returns just the transaction outcome.
    pub fn outcome(&self) -> &ExecutionOutcomeWithIdView {
        self.details.outcome()
    }

    /// Grab all outcomes after the execution of the transaction. This includes outcomes
    /// from the transaction and all the receipts it generated.
    pub fn outcomes(&self) -> Vec<&ExecutionOutcomeWithIdView> {
        self.details.outcomes()
    }

    /// Grab all outcomes after the execution of the transaction. This includes outcomes
    /// only from receipts generated by this transaction.
    pub fn receipt_outcomes(&self) -> &[ExecutionOutcomeWithIdView] {
        self.details.receipt_outcomes()
    }

    /// Grab all outcomes that did not succeed the execution of this transaction. This
    /// will also include the failures from receipts as well.
    pub fn failures(&self) -> Vec<&ExecutionOutcomeWithIdView> {
        self.details.failures()
    }

    /// Just like `failures`, grab only failed receipt outcomes.
    pub fn receipt_failures(&self) -> Vec<&ExecutionOutcomeWithIdView> {
        self.details.receipt_failures()
    }

    /// Grab all logs from both the transaction and receipt outcomes.
    pub fn logs(&self) -> Vec<&str> {
        self.details.logs()
    }

    pub fn status(&self) -> &FinalExecutionStatus {
        &self.status
    }
}

impl ExecutionSuccess {
    /// Deserialize an instance of type `T` from bytes of JSON text sourced from the
    /// execution result of this call. This conversion can fail if the structure of
    /// the internal state does not meet up with [`serde::de::DeserializeOwned`]'s
    /// requirements.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        self.value.json()
    }

    /// Deserialize an instance of type `T` from bytes sourced from the execution
    /// result. This conversion can fail if the structure of the internal state does
    /// not meet up with [`borsh::BorshDeserialize`]'s requirements.
    pub fn borsh<T: borsh::BorshDeserialize>(&self) -> Result<T> {
        self.value.borsh()
    }

    /// Grab the underlying raw bytes returned from calling into a contract's function.
    /// If we want to deserialize these bytes into a rust datatype, use [`ExecutionResult::json`]
    /// or [`ExecutionResult::borsh`] instead.
    pub fn raw_bytes(&self) -> Result<Vec<u8>> {
        self.value.raw_bytes()
    }
}

impl<T> ExecutionResult<T> {
    /// Returns just the transaction outcome.
    pub fn outcome(&self) -> &ExecutionOutcomeWithIdView {
        &self.details.transaction_outcome
    }

    /// Grab all outcomes after the execution of the transaction. This includes outcomes
    /// from the transaction and all the receipts it generated.
    pub fn outcomes(&self) -> Vec<&ExecutionOutcomeWithIdView> {
        let mut outcomes = vec![self.outcome()];
        outcomes.extend(self.receipt_outcomes());
        outcomes
    }

    /// Grab all outcomes after the execution of the transaction. This includes outcomes
    /// only from receipts generated by this transaction.
    pub fn receipt_outcomes(&self) -> &[ExecutionOutcomeWithIdView] {
        &self.details.receipts_outcome
    }

    /// Grab all outcomes that did not succeed the execution of this transaction. This
    /// will also include the failures from receipts as well.
    pub fn failures(&self) -> Vec<&ExecutionOutcomeWithIdView> {
        let mut failures = Vec::new();
        if matches!(
            self.details.transaction_outcome.outcome.status,
            ExecutionStatusView::Failure(_)
        ) {
            failures.push(&self.details.transaction_outcome);
        }
        failures.extend(self.receipt_failures());
        failures
    }

    /// Just like `failures`, grab only failed receipt outcomes.
    pub fn receipt_failures(&self) -> Vec<&ExecutionOutcomeWithIdView> {
        self.receipt_outcomes()
            .iter()
            .filter(|receipt| matches!(receipt.outcome.status, ExecutionStatusView::Failure(_)))
            .collect()
    }

    /// Grab all logs from both the transaction and receipt outcomes.
    pub fn logs(&self) -> Vec<&str> {
        self.outcomes()
            .into_iter()
            .flat_map(|outcome| &outcome.outcome.logs)
            .map(String::as_str)
            .collect()
    }
}

/// Value type returned from an [`ExecutionOutcome`] or receipt result. This value
/// can be converted into the underlying Rust datatype, or directly grab the raw
/// bytes associated to the value.
#[derive(Debug)]
pub struct Value {
    repr: String,
}

impl Value {
    fn from_string(value: String) -> Self {
        Self { repr: value }
    }

    /// Deserialize an instance of type `T` from bytes of JSON text sourced from the
    /// execution result of this call. This conversion can fail if the structure of
    /// the internal state does not meet up with [`serde::de::DeserializeOwned`]'s
    /// requirements.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        let buf = self.raw_bytes()?;
        Ok(serde_json::from_slice(&buf)?)
    }

    /// Deserialize an instance of type `T` from bytes sourced from the execution
    /// result. This conversion can fail if the structure of the internal state does
    /// not meet up with [`borsh::BorshDeserialize`]'s requirements.
    pub fn borsh<T: borsh::BorshDeserialize>(&self) -> Result<T> {
        let buf = self.raw_bytes()?;
        Ok(borsh::BorshDeserialize::try_from_slice(&buf)?)
    }

    /// Grab the underlying raw bytes returned from calling into a contract's function.
    /// If we want to deserialize these bytes into a rust datatype, use [`json`]
    /// or [`borsh`] instead.
    ///
    /// [`json`]: Value::json
    /// [`borsh`]: Value::borsh
    pub fn raw_bytes(&self) -> Result<Vec<u8>> {
        Ok(general_purpose::STANDARD.decode(&self.repr)?)
    }
}
