import { Inject } from "cli-program-lib/decorators/inject.decorator"
import { Injectable } from "cli-program-lib/decorators/injectable.decorator"
import { Logger } from "cli-program-lib/logging/logger"

@Injectable()
export class Performance {

    @Inject(Logger)
    private logger: Logger

    private measures: { [name: string]: { start: number, end: number} } = {}

    public measureStart(name: string): void {
        this.measures[name] = { start: performance.now(), end: undefined }
    }

    public measureEnd(name: string): void {
        this.measures[name].end = performance.now()
    }

    public logMeasures(): void {
        this.logger.debug("Performance measures:")
        for (let name in this.measures) {
            this.logger.debug(name + ": " + this.getMeasure(name) + "ms")
        }
    }

    public getMeasure(name: string): number {
        let measure = this.measures[name]
        return measure.end - measure.start
    }

    public getMeasures(): { [name: string]: number } {
        let result: { [name: string]: number } = {}
        for (let name in this.measures) {
            result[name] = this.getMeasure(name)
        }
        return result
    }

}