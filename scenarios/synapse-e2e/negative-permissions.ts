import assert from 'node:assert/strict'
import {
  DeletePieceError,
  TerminateServiceError,
  WaitForTerminateServiceNotFoundError,
  WaitForTerminateServiceRejectedError,
} from '@filoz/synapse-core/errors'
import * as SP from '@filoz/synapse-core/sp'
import { getRail } from '@filoz/synapse-core/pay'
import { getActivePieceCount } from '@filoz/synapse-core/pdp-verifier'
import { getPdpDataSet } from '@filoz/synapse-core/warm-storage'
import { waitForTransactionReceipt } from 'viem/actions'
import { createSynapse, prepareAccount, readAccountState } from './account.ts'
import { freshMetadata, resolveEnvironment } from './environment.ts'
import { fileSize, uploadFile } from './storage.ts'

const NEGATIVE_TERMINATION_OBSERVATION_MS = 15_000

type DataSetSnapshot = {
  dataSetId: bigint
  pieceId: bigint
  clientDataSetId: bigint
  serviceURL: string
  live: boolean
  activePieceCount: bigint
  rail: { railId: bigint; endEpoch: bigint; paymentRate: bigint; lockupPeriod: bigint }
  payment: { funds: bigint; lockupRate: bigint }
}

async function snapshot(
  synapse: ReturnType<typeof createSynapse>,
  dataSetId: bigint,
  pieceId: bigint
): Promise<DataSetSnapshot> {
  const dataSet = await getPdpDataSet(synapse.client, { dataSetId })
  if (dataSet == null) throw new Error(`Data set ${dataSetId} is not readable`)
  const [activePieceCount, rail, payment] = await Promise.all([
    getActivePieceCount(synapse.client, { dataSetId }),
    getRail(synapse.client, { railId: dataSet.pdpRailId }),
    readAccountState(synapse),
  ])
  return {
    dataSetId,
    pieceId,
    clientDataSetId: dataSet.clientDataSetId,
    serviceURL: dataSet.provider.pdp.serviceURL,
    live: dataSet.live,
    activePieceCount,
    rail: {
      railId: dataSet.pdpRailId,
      endEpoch: rail.endEpoch,
      paymentRate: rail.paymentRate,
      lockupPeriod: rail.lockupPeriod,
    },
    payment: {
      funds: payment.funds,
      lockupRate: payment.lockupRate,
    },
  }
}

async function createPermissionTarget(
  owner: ReturnType<typeof createSynapse>,
  filePath: string,
  label: string
): Promise<DataSetSnapshot> {
  const { result } = await uploadFile(owner, filePath, freshMetadata(`negative-permissions-${label}`), 1)
  const copy = result.copies[0]
  assert(copy != null, `Expected one live data set for ${label}`)
  return snapshot(owner, copy.dataSetId, copy.pieceId)
}

function assertStableAfterRejectedRequest(before: DataSetSnapshot, after: DataSetSnapshot, label: string): void {
  assert.equal(after.live, true, `${label} mutated data set liveness`)
  assert.equal(after.activePieceCount, before.activePieceCount, `${label} mutated active piece count`)
  assert.deepEqual(after.rail, before.rail, `${label} mutated rail identity, rate, lockup period, or end epoch`)
  assert.deepEqual(after.payment, before.payment, `${label} mutated payment funds or lockup rate`)
}

async function assertTerminationRejected(
  operation: () => Promise<SP.terminateService.OutputType>,
  label: string
): Promise<void> {
  try {
    const { statusUrl } = await operation()
    await SP.waitForTerminateService({
      statusUrl,
      timeout: NEGATIVE_TERMINATION_OBSERVATION_MS,
      pollInterval: 1000,
    })
  } catch (error) {
    if (
      TerminateServiceError.is(error) ||
      WaitForTerminateServiceRejectedError.is(error) ||
      WaitForTerminateServiceNotFoundError.is(error)
    ) {
      console.log(`Rejected as expected: ${label}: ${error instanceof Error ? error.message : String(error)}`)
      return
    }
    if (error instanceof Error && error.name === 'TimeoutError') {
      console.log(`No successful termination observed for ${label} after bounded wait`)
      return
    }
    throw error
  }
  throw new Error(`${label} unexpectedly terminated the data set`)
}

async function assertDeletionRejected(
  synapse: ReturnType<typeof createSynapse>,
  operation: () => Promise<SP.deletePiece.OutputType>,
  label: string
): Promise<void> {
  try {
    const { hash } = await operation()
    const receipt = await waitForTransactionReceipt(synapse.client, { hash })
    assert.equal(receipt.status, 'reverted', `${label} transaction unexpectedly succeeded`)
    console.log(`Rejected as expected: ${label}: reverted tx ${hash}`)
  } catch (error) {
    if (DeletePieceError.is(error)) {
      console.log(`Rejected as expected: ${label}: ${error instanceof Error ? error.message : String(error)}`)
      return
    }
    throw error
  }
}

async function main(): Promise<void> {
  const ownerEnvironment = resolveEnvironment({ defaultUserIndex: 0, requireFiles: true })
  if (ownerEnvironment.filePaths.length !== 1) throw new Error('negative-permissions.ts accepts exactly one file path')
  const owner = createSynapse(ownerEnvironment)
  const intruder = createSynapse(resolveEnvironment({ defaultUserIndex: 1 }))
  const [filePath] = ownerEnvironment.filePaths

  await prepareAccount(owner, (await fileSize(filePath)) * 4n)

  const nonOwnerTerminate = await createPermissionTarget(owner, filePath, 'non-owner-terminate')
  await assertTerminationRejected(
    () =>
      SP.terminateService(intruder.client, {
        serviceURL: nonOwnerTerminate.serviceURL,
        dataSetId: nonOwnerTerminate.dataSetId,
      }),
    'non-owner relayed termination'
  )
  assertStableAfterRejectedRequest(
    nonOwnerTerminate,
    await snapshot(owner, nonOwnerTerminate.dataSetId, nonOwnerTerminate.pieceId),
    'non-owner relayed termination'
  )

  const malformedTerminate = await createPermissionTarget(owner, filePath, 'malformed-terminate')
  await assertTerminationRejected(
    () =>
      SP.terminateServiceApiRequest({
        serviceURL: malformedTerminate.serviceURL,
        dataSetId: malformedTerminate.dataSetId,
        extraData: '0x',
      }),
    'malformed relayed termination data'
  )
  assertStableAfterRejectedRequest(
    malformedTerminate,
    await snapshot(owner, malformedTerminate.dataSetId, malformedTerminate.pieceId),
    'malformed relayed termination data'
  )

  const nonOwnerDeletion = await createPermissionTarget(owner, filePath, 'non-owner-deletion')
  await assertDeletionRejected(
    owner,
    () =>
      SP.schedulePieceDeletion(intruder.client, {
        serviceURL: nonOwnerDeletion.serviceURL,
        dataSetId: nonOwnerDeletion.dataSetId,
        clientDataSetId: nonOwnerDeletion.clientDataSetId,
        pieceId: nonOwnerDeletion.pieceId,
      }),
    'non-owner piece deletion'
  )
  assertStableAfterRejectedRequest(
    nonOwnerDeletion,
    await snapshot(owner, nonOwnerDeletion.dataSetId, nonOwnerDeletion.pieceId),
    'non-owner piece deletion'
  )

  const malformedDeletion = await createPermissionTarget(owner, filePath, 'malformed-deletion')
  await assertDeletionRejected(
    owner,
    () =>
      SP.deletePiece({
        serviceURL: malformedDeletion.serviceURL,
        dataSetId: malformedDeletion.dataSetId,
        pieceId: malformedDeletion.pieceId,
        extraData: '0x',
      }),
    'malformed piece deletion data'
  )
  assertStableAfterRejectedRequest(
    malformedDeletion,
    await snapshot(owner, malformedDeletion.dataSetId, malformedDeletion.pieceId),
    'malformed piece deletion data'
  )

  console.log('=== SUCCESS: rejected permission requests left independent data sets unchanged ===')
}

main().catch((error: unknown) => {
  console.error(error)
  process.exitCode = 1
})
