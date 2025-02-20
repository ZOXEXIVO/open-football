import { Component } from '@angular/core';
import { ActivatedRoute } from '@angular/router';
import { UntilDestroy, untilDestroyed } from '@ngneat/until-destroy';
import { TitleService } from 'src/app/shared/services/title.service';
import { CountryDto, CountryService } from '../services/country.service';
import {PageComponent} from "../../page.component";
import {TopHeaderService} from "../../../shared/top-header/services/top.header.service";
import {LeftMenuService} from "../../../shared/left-menu/services/left.menu.service";
import {ProcessService} from "../../../shared/process/services/process.service";

@UntilDestroy()
@Component({
    templateUrl: './country.get.component.html',
    standalone: false,
    styleUrls: ['./country.get.component.scss']
})
export class CountryGetComponent extends PageComponent {
  public country: CountryDto | null = null;

  constructor(private leftMenuService: LeftMenuService,
    private service: CountryService,
    private topHeaderService: TopHeaderService,
    private route: ActivatedRoute,
    private titleService: TitleService,
    private processService: ProcessService) {
    super(processService);
  }

  override onDataRefresh(): void {
    this.leftMenuService.setMenu([
      { items: [{ url: '/', title: 'Home', icon: 'fa-home' }] }
    ]);

    this.route.params.subscribe(params => {
      this.service.get(params["slug"]).pipe(untilDestroyed(this)).subscribe(countryData => {
        this.country = countryData;

        this.titleService.setTitle(countryData.name + ', ' + countryData.continent_name);
        this.topHeaderService.setContent(countryData.name, countryData.continent_name, '/countries');
      });
    });
  }
}
