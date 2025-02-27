use eyre::{Result, WrapErr};
use namada_core::borsh::{BorshDeserialize, BorshSerialize, BorshSerializeExt};
use namada_core::hints;
use namada_core::types::storage::Key;
use namada_core::types::voting_power::FractionalVotingPower;
use namada_state::{DBIter, PrefixIter, StorageHasher, WlStorage, DB};
use namada_storage::{StorageRead, StorageWrite};

use super::{EpochedVotingPower, EpochedVotingPowerExt, Tally, Votes};
use crate::storage::vote_tallies;

pub fn write<D, H, T>(
    wl_storage: &mut WlStorage<D, H>,
    keys: &vote_tallies::Keys<T>,
    body: &T,
    tally: &Tally,
    already_present: bool,
) -> Result<()>
where
    D: 'static + DB + for<'iter> DBIter<'iter> + Sync,
    H: 'static + StorageHasher + Sync,
    T: BorshSerialize,
{
    wl_storage.write_bytes(&keys.body(), &body.serialize_to_vec())?;
    wl_storage.write_bytes(&keys.seen(), &tally.seen.serialize_to_vec())?;
    wl_storage
        .write_bytes(&keys.seen_by(), &tally.seen_by.serialize_to_vec())?;
    wl_storage.write_bytes(
        &keys.voting_power(),
        &tally.voting_power.serialize_to_vec(),
    )?;
    if !already_present {
        // add the current epoch for the inserted event
        wl_storage.write_bytes(
            &keys.voting_started_epoch(),
            &wl_storage.storage.get_current_epoch().0.serialize_to_vec(),
        )?;
    }
    Ok(())
}

/// Delete a tally from storage, and return the associated value of
/// type `T` being voted on, in case it has accumulated more than 1/3
/// of fractional voting power behind it.
#[must_use = "The storage value returned by this function must be used"]
pub fn delete<D, H, T>(
    wl_storage: &mut WlStorage<D, H>,
    keys: &vote_tallies::Keys<T>,
) -> Result<Option<T>>
where
    D: 'static + DB + for<'iter> DBIter<'iter> + Sync,
    H: 'static + StorageHasher + Sync,
    T: BorshDeserialize,
{
    let opt_body = {
        let voting_power: EpochedVotingPower =
            super::read::value(wl_storage, &keys.voting_power())?;

        if hints::unlikely(
            voting_power.fractional_stake(wl_storage)
                > FractionalVotingPower::ONE_THIRD,
        ) {
            let body: T = super::read::value(wl_storage, &keys.body())?;
            Some(body)
        } else {
            None
        }
    };
    wl_storage.delete(&keys.body())?;
    wl_storage.delete(&keys.seen())?;
    wl_storage.delete(&keys.seen_by())?;
    wl_storage.delete(&keys.voting_power())?;
    wl_storage.delete(&keys.voting_started_epoch())?;
    Ok(opt_body)
}

pub fn read<D, H, T>(
    wl_storage: &WlStorage<D, H>,
    keys: &vote_tallies::Keys<T>,
) -> Result<Tally>
where
    D: 'static + DB + for<'iter> DBIter<'iter> + Sync,
    H: 'static + StorageHasher + Sync,
{
    let seen: bool = super::read::value(wl_storage, &keys.seen())?;
    let seen_by: Votes = super::read::value(wl_storage, &keys.seen_by())?;
    let voting_power: EpochedVotingPower =
        super::read::value(wl_storage, &keys.voting_power())?;

    Ok(Tally {
        voting_power,
        seen_by,
        seen,
    })
}

pub fn iter_prefix<'a, D, H>(
    wl_storage: &'a WlStorage<D, H>,
    prefix: &Key,
) -> Result<PrefixIter<'a, D>>
where
    D: 'static + DB + for<'iter> DBIter<'iter> + Sync,
    H: 'static + StorageHasher + Sync,
{
    wl_storage
        .iter_prefix(prefix)
        .context("Failed to iterate over the given storage prefix")
}

#[inline]
pub fn read_body<D, H, T>(
    wl_storage: &WlStorage<D, H>,
    keys: &vote_tallies::Keys<T>,
) -> Result<T>
where
    D: 'static + DB + for<'iter> DBIter<'iter> + Sync,
    H: 'static + StorageHasher + Sync,
    T: BorshDeserialize,
{
    super::read::value(wl_storage, &keys.body())
}

#[inline]
pub fn maybe_read_seen<D, H, T>(
    wl_storage: &WlStorage<D, H>,
    keys: &vote_tallies::Keys<T>,
) -> Result<Option<bool>>
where
    D: 'static + DB + for<'iter> DBIter<'iter> + Sync,
    H: 'static + StorageHasher + Sync,
    T: BorshDeserialize,
{
    super::read::maybe_value(wl_storage, &keys.seen())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use assert_matches::assert_matches;
    use namada_core::types::ethereum_events::EthereumEvent;

    use super::*;
    use crate::test_utils;

    #[test]
    fn test_delete_expired_tally() {
        let (mut wl_storage, _) = test_utils::setup_default_storage();
        let (validator, validator_voting_power) =
            test_utils::default_validator();

        let event = EthereumEvent::TransfersToNamada {
            nonce: 0.into(),
            transfers: vec![],
        };
        let keys = vote_tallies::Keys::from(&event);

        // write some random ethereum event's tally to storage
        // with >1/3 voting power behind it
        let mut tally = Tally {
            voting_power: EpochedVotingPower::from([(
                0.into(),
                // store only half of the available voting power,
                // which is >1/3 but <=2/3
                FractionalVotingPower::HALF * validator_voting_power,
            )]),
            seen_by: BTreeMap::from([(validator, 1.into())]),
            seen: false,
        };
        assert!(write(&mut wl_storage, &keys, &event, &tally, false).is_ok());

        // delete the tally and check that the body is returned
        let opt_body = delete(&mut wl_storage, &keys).unwrap();
        assert_matches!(opt_body, Some(e) if e == event);

        // now, we write another tally, with <=1/3 voting power
        tally.voting_power =
            EpochedVotingPower::from([(0.into(), 1u64.into())]);
        assert!(write(&mut wl_storage, &keys, &event, &tally, false).is_ok());

        // delete the tally and check that no body is returned
        let opt_body = delete(&mut wl_storage, &keys).unwrap();
        assert_matches!(opt_body, None);
    }

    #[test]
    fn test_write_tally() {
        let (mut wl_storage, _) = test_utils::setup_default_storage();
        let (validator, validator_voting_power) =
            test_utils::default_validator();
        let event = EthereumEvent::TransfersToNamada {
            nonce: 0.into(),
            transfers: vec![],
        };
        let keys = vote_tallies::Keys::from(&event);
        let tally = Tally {
            voting_power: EpochedVotingPower::from([(
                0.into(),
                validator_voting_power,
            )]),
            seen_by: BTreeMap::from([(validator, 10.into())]),
            seen: false,
        };

        let result = write(&mut wl_storage, &keys, &event, &tally, false);

        assert!(result.is_ok());
        let body = wl_storage.read_bytes(&keys.body()).unwrap();
        assert_eq!(body, Some(event.serialize_to_vec()));
        let seen = wl_storage.read_bytes(&keys.seen()).unwrap();
        assert_eq!(seen, Some(tally.seen.serialize_to_vec()));
        let seen_by = wl_storage.read_bytes(&keys.seen_by()).unwrap();
        assert_eq!(seen_by, Some(tally.seen_by.serialize_to_vec()));
        let voting_power = wl_storage.read_bytes(&keys.voting_power()).unwrap();
        assert_eq!(voting_power, Some(tally.voting_power.serialize_to_vec()));
        let epoch =
            wl_storage.read_bytes(&keys.voting_started_epoch()).unwrap();
        assert_eq!(
            epoch,
            Some(wl_storage.storage.get_current_epoch().0.serialize_to_vec())
        );
    }

    #[test]
    fn test_read_tally() {
        let (mut wl_storage, _) = test_utils::setup_default_storage();
        let (validator, validator_voting_power) =
            test_utils::default_validator();
        let event = EthereumEvent::TransfersToNamada {
            nonce: 0.into(),
            transfers: vec![],
        };
        let keys = vote_tallies::Keys::from(&event);
        let tally = Tally {
            voting_power: EpochedVotingPower::from([(
                0.into(),
                validator_voting_power,
            )]),
            seen_by: BTreeMap::from([(validator, 10.into())]),
            seen: false,
        };
        wl_storage
            .write_bytes(&keys.body(), &event.serialize_to_vec())
            .unwrap();
        wl_storage
            .write_bytes(&keys.seen(), &tally.seen.serialize_to_vec())
            .unwrap();
        wl_storage
            .write_bytes(&keys.seen_by(), &tally.seen_by.serialize_to_vec())
            .unwrap();
        wl_storage
            .write_bytes(
                &keys.voting_power(),
                &tally.voting_power.serialize_to_vec(),
            )
            .unwrap();
        wl_storage
            .write_bytes(
                &keys.voting_started_epoch(),
                &wl_storage.storage.get_block_height().0.serialize_to_vec(),
            )
            .unwrap();

        let result = read(&wl_storage, &keys);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), tally);
    }
}
