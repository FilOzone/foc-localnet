import assert from 'node:assert/strict'
import { getRail } from '@filoz/synapse-core/pay'
import { getPriceList, getPdpDataSet } from '@filoz/synapse-core/warm-storage'
import { createSynapse, prepareAccount, readAccountState } from './account.ts'
import { freshMetadata, resolveEnvironment } from './environment.ts'
import { fileSize, uploadFile } from './storage.ts'

const delay = (milliseconds: number) => new Promise<void>((resolve) => setTimeout(resolve, milliseconds))

async function waitForTerminatedRail(synapse: ReturnType<typeof createSynapse>, dataSetId: bigint): Promise<bigint> {
  for (let attempt = 0; attempt < 30; attempt++) {
    const dataSet = await getPdpDataSet(synapse.client, { dataSetId })
    if (dataSet != null) {
      const rail = await getRail(synapse.client, { railId: dataSet.pdpRailId })
      if (rail.endEpoch > 0n) return rail.endEpoch
    }
    await delay(1000)
  }
  throw new Error(`Data set ${dataSetId} payment rail did not become terminated`)
}

async function main(): Promise<void> {
  const environment = resolveEnvironment({ defaultUserIndex: 0, requireFiles: true })
  if (environment.filePaths.length !== 1) throw new Error('termination-controls.ts accepts exactly one file path')
  const [filePath] = environment.filePaths
  const synapse = createSynapse(environment)
  await prepareAccount(synapse, (await fileSize(filePath)) * 2n)

  const { result } = await uploadFile(synapse, filePath, freshMetadata('termination-controls'), 2)
  assert.equal(result.copies.length, 2, 'Expected two independent data sets')
  const [relayedCopy, directCopy] = result.copies
  const priceList = await getPriceList(synapse.client)

  const beforeRelayed = await readAccountState(synapse)
  const relayed = await synapse.storage.terminateService({
    dataSetId: relayedCopy.dataSetId,
    onSubmitted: (hash) => console.log(`SP-relayed termination submitted: ${hash}`),
  })
  assert.equal(relayed.dataSetId, relayedCopy.dataSetId, 'Relayed termination returned the wrong data set')
  assert(relayed.endEpoch > 0n, 'Relayed termination returned an empty end epoch')
  const relayedRailEndEpoch = await waitForTerminatedRail(synapse, relayedCopy.dataSetId)
  assert.equal(relayedRailEndEpoch, relayed.endEpoch, 'Relayed termination result differs from the rail end epoch')
  const afterRelayed = await readAccountState(synapse)
  assert(
    beforeRelayed.funds - afterRelayed.funds >= priceList.fees.terminateFee,
    'SP-relayed termination did not charge at least the configured termination fee'
  )

  const beforeDirect = await readAccountState(synapse)
  const direct = await synapse.storage.terminateService({
    dataSetId: directCopy.dataSetId,
    skipProvider: true,
    onSubmitted: (hash) => console.log(`Direct termination submitted: ${hash}`),
  })
  assert.equal(direct.dataSetId, directCopy.dataSetId, 'Direct termination returned the wrong data set')
  assert(direct.txHash != null, 'Direct termination did not submit an on-chain transaction')
  assert(direct.endEpoch > 0n, 'Direct termination returned an empty end epoch')
  const directRailEndEpoch = await waitForTerminatedRail(synapse, directCopy.dataSetId)
  assert.equal(directRailEndEpoch, direct.endEpoch, 'Direct termination result differs from the rail end epoch')
  const afterDirect = await readAccountState(synapse)
  assert.equal(
    beforeDirect.funds - afterDirect.funds,
    0n,
    'Direct termination charged the SP-mediated termination fee'
  )
  console.log('=== SUCCESS: relayed and direct termination controls observed; no cleanup wait performed ===')
}

main().catch((error: unknown) => {
  console.error(error)
  process.exitCode = 1
})
