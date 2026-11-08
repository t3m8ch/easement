import 'router-lib';
import { secret } from '../server/secret';
declare module 'router-lib' { interface Register { router: typeof secret } }
export const app = 1;
