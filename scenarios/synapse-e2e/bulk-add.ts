import assert from 'node:assert/strict'
import * as SP from '@filoz/synapse-core/sp'
import { findPieceIdsByCidCall, getActivePieceCount } from '@filoz/synapse-core/pdp-verifier'
import { getPdpDataSet } from '@filoz/synapse-core/warm-storage'
import { readContract } from 'viem/actions'
import { createSynapse, prepareAccount } from './account.ts'
import { freshMetadata, resolveEnvironment } from './environment.ts'
import { fileSize, uploadFile } from './storage.ts'

const REQUIRED_PIECES = 40
const SMALL_PIECE_BYTES = 64 * 1024

function fixtureFor(pieceNumber: number): File {
  const bytes = Buffer.alloc(SMALL_PIECE_BYTES, pieceNumber % 251)
  bytes.write(`foc-devnet-bulk-add-${pieceNumber}`, 0, 'utf8')
  return new File([bytes], `bulk-${pieceNumber.toString().padStart(3, '0')}.bin`, {
    type: 'application/octet-stream',
  })
}

async function main(): Promise<void> {
  const environment = resolveEnvironment({ defaultUserIndex: 0, requireFiles: true })
  if (environment.filePaths.length !== 1) throw new Error('bulk-add.ts accepts exactly one bootstrap file path')
  const [bootstrapPath] = environment.filePaths
  const synapse = createSynapse(environment)

  // Prepare enough headroom for creation plus the maximum-size add-pieces batch.
  await prepareAccount(synapse, (await fileSize(bootstrapPath)) + BigInt(SMALL_PIECE_BYTES * REQUIRED_PIECES))
  const { result } = await uploadFile(synapse, bootstrapPath, freshMetadata('bulk-add'), 1)
  const copy = result.copies[0]
  assert(copy != null, 'Expected a bootstrap data set')
  const dataSet = await getPdpDataSet(synapse.client, { dataSetId: copy.dataSetId })
  assert(dataSet != null && dataSet.live, `Bootstrap data set ${copy.dataSetId} is not live`)
  const initialPieceCount = await getActivePieceCount(synapse.client, { dataSetId: copy.dataSetId })

  const files = Array.from({ length: REQUIRED_PIECES }, (_, index) => fixtureFor(index + 1))
  const added = await SP.upload(synapse.client, {
    dataSetId: copy.dataSetId,
    data: files,
  })
  assert.equal(added.pieces.length, REQUIRED_PIECES, `Expected ${REQUIRED_PIECES} submitted pieces`)
  const pieceCids = added.pieces.map((piece) => piece.pieceCid)
  assert.equal(
    new Set(pieceCids.map((pieceCid) => pieceCid.toString())).size,
    REQUIRED_PIECES,
    'Piece CIDs are not unique'
  )

  const confirmed = await SP.waitForAddPieces({ statusUrl: added.statusUrl, timeout: 180_000, pollInterval: 1000 })
  assert.equal(confirmed.piecesAdded, true, 'Batched pieces were not added')
  assert.equal(confirmed.confirmedPieceIds.length, REQUIRED_PIECES, 'Batched piece confirmation was incomplete')
  console.log(`Added ${REQUIRED_PIECES} pieces in one batch: tx=${confirmed.txHash}`)

  const activePieceCount = await getActivePieceCount(synapse.client, { dataSetId: copy.dataSetId })
  assert.equal(
    activePieceCount,
    initialPieceCount + BigInt(REQUIRED_PIECES),
    'On-chain active-piece count differs from successfully submitted pieces'
  )
  await Promise.all(
    pieceCids.map(async (pieceCid) => {
      const ids = await readContract(
        synapse.client,
        findPieceIdsByCidCall({
          chain: synapse.client.chain,
          dataSetId: copy.dataSetId,
          pieceCid,
          startPieceId: 0n,
          limit: 2n,
        })
      )
      assert.equal(ids.length, 1, `Submitted piece ${pieceCid} is not discoverable on-chain`)
    })
  )
  console.log(`=== SUCCESS: ${REQUIRED_PIECES} distinct pieces are discoverable on data set ${copy.dataSetId} ===`)
}

main().catch((error: unknown) => {
  console.error(error)
  process.exitCode = 1
})
