import { fileURLToPath } from 'node:url';
import { prepareReceiptAssets } from './receipt-assets.mjs';

const result = await prepareReceiptAssets(fileURLToPath(new URL('..', import.meta.url)), { download: true });
console.log(result ? 'Verified immutable receipt compiler assets: ' + result.profile.release.tag : 'Legacy-only build: no receipt compiler assets requested');
